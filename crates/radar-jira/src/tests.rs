//! The plugin through the real host: the screen, ticket pages, settings,
//! menus, typing, mouse, refreshing and the pause.

#[path = "settings_polish_tests.rs"]
mod settings_polish;

#[path = "sync_incremental_tests.rs"]
mod sync_incremental;

#[path = "rate_hold_tests.rs"]
mod rate_hold;

#[path = "page_stored_tests.rs"]
mod page_stored;

#[path = "code_reread_tests.rs"]
mod code_reread;

#[path = "filter_tests.rs"]
mod filter;


use super::*;
use crate::model::fixtures::ticket;
use crate::model::{Category, Link, Target as LinkTarget};
use radar_core::host::{Event, HostEffect};

pub(crate) use crate::demo::*;

#[test]
fn journal_ticket_scope_is_explicit_and_does_not_guess_the_first_site() {
    let mut p = sample();
    let one = join::ticket(&p, "acme", "ABC-12").with("title", "private summary");
    let a = p.journal_subject(&one).unwrap();
    let mut other = p.cfg.site("acme").unwrap().clone();
    other.name = "other".into();
    other.url = "https://other.atlassian.net".into();
    p.cfg.sites.push(other);
    let b = p.journal_subject(&join::ticket(&p, "other", "ABC-12")).unwrap();
    assert_eq!(a.key.id, b.key.id);
    assert_ne!(a.key.instance, b.key.instance);
    assert!(!format!("{a:?}").contains("private"));
    assert!(p.journal_subject(&radar_core::Subject::new("ticket", "ABC-12", "x")).is_none());
    assert!(p.journal_subject(&one.with_url("https://foreign.example/browse/ABC-12")).is_none());
}

/// Supply a fixture only through a legitimately pending page read.
pub(crate) fn detail_read(p: &mut JiraPlugin, site: &str, key: &str) -> page::Read {
    let _ = page::fetch(p, site, key, true);
    p.details[&detail_key(site, key)].request.expect("pending page read")
}

pub(crate) fn complete_detail(p: &mut JiraPlugin, site: &str, key: &str, result: Result<Detail, String>) {
    let read = detail_read(p, site, key);
    assert!(page::on_detail(p, site, key, read, result.map(|detail| client::DetailRead {
        detail,
        ..Default::default()
    })));
}

/// One with references only, as the team's tools set it up.
const MCP_REFS: &str = r#"{ "mcpServers": { "tools": { "env": {
    "JIRA_BASE_URL": "https://delta.atlassian.net",
    "JIRA_EMAIL": "${JIRA_EMAIL}",
    "JIRA_TOKEN": "${JIRA_TOKEN}",
    "JIRA_PROJECT_KEY": "DEL",
    "JIRA_ACCEPTANCE_CRITERIA_FIELD": "customfield_10044" } } } }"#;


/// ABC-12's page data.
pub(crate) fn detail() -> Detail {
    let p = sample();
    let t = p.data["acme"].tickets["ABC-12"].clone();
    Detail {
        ticket: Ticket {
            description: doc("## Why\nThe dashboard needs a count of requests per day.\n\n```\nGET /stats?days=7\n```"),
            ..t
        },
        sprint: Some("Sprint 41".into()),
        links: vec![
            Link {
                relation: "parent".into(),
                title: "Dashboard for the stats".into(),
                target: LinkTarget::Ticket { key: "ABC-1".into(), status: "In Progress".into(), category: Category::Doing },
            },
            Link {
                relation: "child".into(),
                title: "Count requests per route".into(),
                target: LinkTarget::Ticket { key: "ABC-13".into(), status: "In Progress".into(), category: Category::Doing },
            },
            Link {
                relation: "blocks".into(),
                title: "Ship the dashboard".into(),
                target: LinkTarget::Ticket { key: "ABC-15".into(), status: "To Do".into(), category: Category::Todo },
            },
            Link {
                relation: "web link".into(),
                title: "Design notes".into(),
                target: LinkTarget::Web { url: "https://example.com/design".into() },
            },
        ],
        comments: vec![
            model::Comment {
                author: "Bob Example".into(),
                created: "2026-10-06T06:30:00.000+0000".into(),
                body: doc("Looks right; one question on the time zone."),
            },
            model::Comment {
                author: "Alice Example".into(),
                created: "2026-10-05T08:00:00.000+0000".into(),
                body: doc("First cut is up."),
            },
        ],
        comments_total: 3,
        code: vec![model::CodeLink {
            id: "!263".into(),
            title: "ABC-12: Add a stats endpoint for the dashboard".into(),
            url: "https://gitlab.com/acme/team-a/api/-/merge_requests/263".into(),
            status: "open".into(),
            repo: "api".into(),
            branch: "abc-12-stats".into(),
        }],
    }
}

#[test]
fn tree_context_selection_requires_a_drawn_row() {
    use radar_core::{Outcome, Pane, Plugin};

    let mut p = sample();
    // Each shown ticket row is a target of its own: its path (printed in
    // braces, the ids joined with `/`).
    let got = shown(&hosted(), 160, 40);
    let id = got
        .lines()
        .filter_map(|l| l.rsplit_once('{').and_then(|(_, t)| t.strip_suffix('}')))
        .find(|t| t.split('/').count() == 3)
        .expect("a ticket row");
    assert!(got.contains("{s:acme/g:acme:In Progress/t:acme:ABC-1}"), "{got}");
    let target = radar_core::view::TreeRow::target_of(&id.split('/').map(str::to_string).collect::<Vec<_>>());
    for _ in 0..2 {
        let context = radar_core::view::Intent::Context { target: target.clone() };
        assert!(matches!(p.intent(SCREEN.id, Pane::Middle, &context), Outcome::Handled(_)));
        assert_eq!(p.tree.borrow().selected(), radar_core::view::TreeRow::path_of(&target).as_slice());
    }
    // A pane's blank space has no menu of its own and keeps the selection.
    let blank = radar_core::view::Intent::Select { target: radar_core::view::Target::new("pane:middle") };
    let _ = p.intent(SCREEN.id, Pane::Middle, &blank);
    assert_eq!(p.tree.borrow().selected(), radar_core::view::TreeRow::path_of(&target).as_slice());
}

pub(crate) fn hosted() -> radar_core::Host {
    let mut host = radar_core::Host::new(vec![Box::new(sample())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(sample());
    host
}

/// The Jira plugin inside a host these tests built.
fn jira(host: &radar_core::Host) -> &JiraPlugin {
    assert_eq!(host.plugins[0].id(), "jira");
    // SAFETY: every host here is built with one plugin, a `JiraPlugin`
    // (checked by its id above); the cast drops only the vtable.
    unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const JiraPlugin) }
}

pub(crate) fn jira_mut(host: &mut radar_core::Host) -> &mut JiraPlugin {
    assert_eq!(host.plugins[0].id(), "jira");
    // SAFETY: as in `jira`.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut JiraPlugin) }
}

/// What shows, as view text (`radar_core::headless::text`): the screen's panes
/// and rows, the open menu and the status line. No painted frame.
pub(crate) fn shown(host: &radar_core::Host, w: u16, h: u16) -> String {
    radar_core::headless::text_of(host, w, h)
}

fn press(host: &mut radar_core::Host, code: KeyCode) -> Vec<HostEffect> {
    let _ = shown(host, 160, 40);
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

/// ↑ on the first row of each pane reaches the top bar; → changes the
/// screen, ↓ comes back to that pane; inside a ticket's page ↑ stays.
#[test]
fn up_from_the_top_of_each_pane_reaches_the_bar_and_down_comes_back() {
    use radar_core::host::View;
    for pane in [Pane::Nav, Pane::Middle, Pane::Detail] {
        let mut host = hosted();
        host.focus = pane;
        press(&mut host, KeyCode::Char('g'));
        press(&mut host, KeyCode::Up);
        assert!(host.bar, "{pane:?}: ↑ at the top reaches the bar");
        press(&mut host, KeyCode::Right);
        assert_eq!(host.view, View::Settings);
        press(&mut host, KeyCode::Left);
        press(&mut host, KeyCode::Down);
        assert_eq!((host.view, host.bar, host.focus), (View::Screen(0), false, pane));
    }
    let mut host = hosted();
    open_ticket(&mut host, "ABC-12");
    press(&mut host, KeyCode::Char('g'));
    press(&mut host, KeyCode::Up);
    assert!(!host.bar, "inside a page");
}

#[test]
fn plus_and_minus_fold_the_tree_and_minus_on_a_ticket_closes_its_status() {
    let mut host = hosted();
    host.focus = Pane::Middle;
    let got = shown(&host, 160, 40);
    assert!(got.split("── status").nth(1).unwrap().contains("+/- expand/collapse"), "{got}");
    for _ in 0..10 {
        if jira(&host).tree.borrow().selected().len() >= 3 {
            break;
        }
        press(&mut host, KeyCode::Down);
    }
    let ticket = jira(&host).tree.borrow().selected().to_vec();
    assert!(ticket.len() >= 3, "{ticket:?}");
    let group = ticket[..ticket.len() - 1].to_vec();
    // The epic has its stories under it: `-` closes it first, then its status.
    press(&mut host, KeyCode::Char('-'));
    assert_eq!(jira(&host).tree.borrow().selected(), ticket.as_slice());
    assert!(!jira(&host).tree.borrow().is_open(&ticket));
    press(&mut host, KeyCode::Char('-'));
    assert_eq!(jira(&host).tree.borrow().selected(), group.as_slice());
    assert!(!jira(&host).tree.borrow().is_open(&group));
    press(&mut host, KeyCode::Char('+'));
    assert!(jira(&host).tree.borrow().is_open(&group));
    press(&mut host, KeyCode::Char('-'));
    press(&mut host, KeyCode::Char('='));
    assert!(jira(&host).tree.borrow().is_open(&group));
    // An already-open group owns `+` as a quiet no-op under the shared
    // contract: it must not change the selected subject, folds, or pane.
    let before = jira(&host).tree.borrow().clone();
    let effects = press(&mut host, KeyCode::Char('+'));
    assert_eq!(*jira(&host).tree.borrow(), before);
    assert_eq!(host.focus, Pane::Middle);
    assert!(effects.is_empty(), "opening an open group must not start other work");
    assert!(host.message.is_none(), "a tree boundary is a quiet no-op");
}

fn typed(host: &mut radar_core::Host, s: &str) {
    for c in s.chars() {
        press(host, KeyCode::Char(c));
    }
}

fn spawned(fx: &[HostEffect]) -> usize {
    fx.iter().filter(|e| matches!(e, HostEffect::Spawn { .. })).count()
}

fn section(host: &mut radar_core::Host, name: &str) {
    host.view = radar_core::host::View::Settings;
    host.section = host.sections.iter().position(|(_, s)| s.name == name).unwrap();
    host.focus = Pane::Middle;
}

/// The middle pane's selected row as view text (it starts with `> `).
fn selected_line(host: &radar_core::Host) -> Option<String> {
    let got = shown(host, 160, 40);
    let middle = got.split("── middle").nth(1)?;
    middle.lines().find(|l| l.starts_with("> ")).map(str::to_string)
}

/// Down the tree until `key` is selected, then ⏎.
fn open_ticket(host: &mut radar_core::Host, key: &str) {
    for _ in 0..20 {
        if selected_line(host).is_some_and(|l| l.contains(&format!(":{key}}}"))) {
            press(host, KeyCode::Enter);
            return;
        }
        press(host, KeyCode::Down);
    }
    panic!("{key} never selected:\n{}", shown(host, 160, 40));
}

// --- the screen ---------------------------------------------------------------------

#[test]
fn the_tree_is_site_then_status_then_tickets_with_children_under_their_parent() {
    let p = sample();
    let nodes = screen::nodes(&p);
    assert_eq!(nodes.len(), 2);
    let acme = &nodes[0];
    let statuses: Vec<&str> = acme.groups.iter().map(|g| g.status.as_str()).collect();
    assert_eq!(statuses, ["In Progress", "In Review", "To Do"], "doing first, then to do");
    let doing = &acme.groups[0];
    assert_eq!(doing.tops.len(), 1, "the epic, its story under it");
    assert_eq!(doing.tops[0].ticket.key, "ABC-1");
    let kids: Vec<&str> = doing.tops[0].children.iter().map(|t| t.ticket.key.as_str()).collect();
    assert_eq!(kids, ["ABC-12", "ABC-13"]);
    assert_eq!(nodes[1].groups[0].tops[0].ticket.key, "XYZ-7");
}

#[test]
fn jira_screen() {
    insta::assert_snapshot!(shown(&hosted(), 160, 40));
}

#[test]
fn the_preview_of_a_ticket_has_its_description_and_acceptance_criteria() {
    let mut host = hosted();
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    let got = shown(&host, 160, 40);
    for want in [
        "acme / ABC / ABC-12",
        "type: Story",
        "parent: ABC-1: Dashboard for the stats",
        "Add a stats endpoint for the dashboard",
        "status: :in_progress: In Progress",
        "assignee: :you: you",
        "Bob Example",
        "1h ago",
        "#backend",
        "DESCRIPTION",
        "What ABC-12 is about.",
        "ACCEPTANCE CRITERIA",
        "• GET /stats answers within 200 ms",
    ] {
        assert!(got.contains(want), "{want:?} missing:\n{got}");
    }
    insta::assert_snapshot!("ticket_preview", got);
}

#[test]
fn views_in_the_rail_are_words_never_jql() {
    let got = shown(&hosted(), 160, 40);
    assert!(got.contains("assigned to you") && got.contains("not done"), "{got}");
    assert!(!got.contains("currentUser") && !got.contains("statusCategory"), "{got}");
}

#[test]
fn moving_in_the_rail_switches_the_view_and_fetches_it() {
    let mut host = hosted();
    press(&mut host, KeyCode::Char('1'));
    let fx = press(&mut host, KeyCode::Down);
    assert_eq!(jira(&host).view, 1);
    assert_eq!(spawned(&fx), 2, "This sprint, from both sites");
    let got = shown(&host, 160, 40);
    assert!(got.contains("in an open sprint"), "{got}");
}

#[test]
fn a_view_that_needs_watched_projects_skips_a_site_without_and_says_so() {
    let mut p = sample();
    p.cfg.sites[1].projects.clear();
    p.view = 1;
    assert!(p.jql_for(1, &p.cfg.views[1].clone()).is_none());
    let fx = p.sync_view(true);
    assert_eq!(fx.len(), 1, "only acme is asked");
    let mut host = radar_core::Host::new(vec![Box::new(sample())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    let got = shown(&host, 160, 40);
    assert!(got.contains("nothing asked of beta"), "{got}");
    assert!(got.contains("nothing to ask here in this view"), "{got}");
}

#[test]
fn typed_words_and_the_picked_assignee_leave_out_sites_with_nothing_found() {
    let mut p = sample();
    let sites = |p: &JiraPlugin| screen::nodes(p).len();
    assert_eq!(sites(&p), 2, "no filter: every site, empty or not");
    p.filter = "export".into();
    assert_eq!(sites(&p), 1, "only the site holding what was found");
    p.filter = "nothing-says-this".into();
    assert_eq!(sites(&p), 0);
    p.filter.clear();
    p.assignee = Some(NOBODY.into());
    assert_eq!(sites(&p), 1, "the assignee narrows the same way");
    p.assignee = None;
    assert_eq!(sites(&p), 2);
}

#[test]
fn the_assignee_menu_picks_one_person_or_nobody() {
    let mut host = hosted();
    press(&mut host, KeyCode::Char('a'));
    let menu = host.menu.as_ref().expect("the assignee menu");
    let labels: Vec<&str> = menu.menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["everyone", "nobody", "Alice Example", "Bob Example"]);
    press(&mut host, KeyCode::Char('n'));
    let keys: Vec<String> = jira(&host).visible().iter().map(|it| it.t.key.clone()).collect();
    assert_eq!(keys, ["ABC-20"]);
    let got = shown(&host, 160, 40);
    assert!(got.lines().any(|l| l.starts_with("ASSIGNED TO") && l.contains("nobody")), "{got}");
    // esc in the tree brings everyone back.
    press(&mut host, KeyCode::Esc);
    assert_eq!(jira(&host).assignee, None);
}

#[test]
fn go_to_jumps_to_a_key_and_opens_one_not_in_the_view() {
    let mut host = hosted();
    press(&mut host, KeyCode::Char(':'));
    assert!(host.plugins[0].capturing());
    typed(&mut host, "abc-20");
    let got = shown(&host, 160, 40);
    assert!(got.contains("at ABC-20 Stats page shows yesterday twice"), "{got}");
    press(&mut host, KeyCode::Enter);
    assert!(!host.plugins[0].capturing());
    assert!(shown(&host, 160, 40).contains("── detail · ABC-20 ──"));
    // A key the view does not have: its page, fetched from the site watching it.
    press(&mut host, KeyCode::Char(':'));
    host.update(Event::Paste("XYZ-99".into()));
    let got = shown(&host, 160, 40);
    assert!(got.contains("not in this view") && !got.contains("opens its page"), "{got}");
    let fx = press(&mut host, KeyCode::Enter);
    assert_eq!(spawned(&fx), 1);
    let page = jira(&host).pages.last().expect("a page");
    assert_eq!((page.site.as_str(), page.key.as_str()), ("beta", "XYZ-99"));
}

#[test]
fn pgdn_from_any_pane_keeps_the_tree_selection_and_moving_shows_the_next_preview() {
    let mut host = hosted();
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    let selected = jira(&host).tree.borrow().selected().to_vec();
    host.update(Event::Key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)));
    assert_eq!(jira(&host).tree.borrow().selected(), selected, "PgDn scrolls the preview, not the tree");
    // In a focused detail pane, ↓ moves through it, not to the next ticket.
    press(&mut host, KeyCode::Char('3'));
    let before = jira(&host).tree.borrow().selected().to_vec();
    host.update(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
    assert_eq!(jira(&host).tree.borrow().selected(), before);
    press(&mut host, KeyCode::Esc);
    press(&mut host, KeyCode::Down);
    let got = shown(&host, 160, 14);
    let path = jira(&host).tree.borrow().selected().to_vec();
    let key = path.last().and_then(|id| id.rsplit(':').next()).unwrap_or_default().to_string();
    assert!(got.contains(&format!("acme / ABC / {key}")), "moving shows that ticket's preview: {got}");
}

#[test]
fn selecting_a_tree_row_selects_it_and_the_assignee_heading_opens_the_menu() {
    use radar_core::view::{Intent, Target};
    let mut host = hosted();
    let select = |host: &mut radar_core::Host, target: &str| {
        let _ = shown(host, 160, 40);
        host.update(Event::Intent(Intent::Select { target: Target::new(target) }));
    };
    select(&mut host, "view:0");
    assert_eq!(host.focus, Pane::Nav, "a press on another pane's row focuses it");
    select(&mut host, "s:acme\u{1f}g:acme:In Progress\u{1f}t:acme:ABC-1");
    assert_eq!(host.focus, Pane::Middle);
    assert_eq!(jira(&host).tree.borrow().selected().len(), 3, "the press selects the ticket row at once");
    select(&mut host, "assignees");
    assert_eq!(host.menu.as_ref().map(|m| m.menu.id.as_str()), Some(ASSIGNEE_MENU));
}

#[test]
fn bulk_copies_every_key_and_link_and_opens_and_closes_the_tree() {
    let mut p = sample();
    let items = screen::bulk(&p);
    assert!(items.iter().all(|i| !matches!(i.key, Some('j' | 'k' | 'q'))));
    let fx = screen::on_bulk(&mut p, "j-keys").unwrap();
    assert!(matches!(&fx[0], Effect::Copy(t) if t.lines().count() == 6 && t.contains("XYZ-7")), "{fx:?}");
    let fx = screen::on_bulk(&mut p, "j-links").unwrap();
    assert!(matches!(&fx[0], Effect::Copy(t) if t.contains("https://acme.atlassian.net/browse/ABC-12")));
    assert_eq!(screen::on_bulk(&mut p, "j-browse").unwrap().len(), 6);
    screen::on_bulk(&mut p, "j-close").unwrap();
    assert!(p.tree.borrow().open.is_empty());
    assert!(screen::on_bulk(&mut p, "s-check-all").is_none(), "settings' own");
}

// --- refreshing ------------------------------------------------------------------

/// List/page-only fixtures retain the legacy missing provider IDs explicitly,
/// so their exact request counts do not include development-panel prefetch.
fn sample_without_provider_ids() -> JiraPlugin {
    let mut p = sample();
    for data in p.data.values_mut() {
        for ticket in data.tickets.values_mut() {
            ticket.id.clear();
        }
    }
    p
}

#[test]
fn tick_asks_each_site_once_and_folds_the_answers() {
    let mut p = sample_without_provider_ids();
    let fx = p.tick(NOW);
    assert_eq!(fx.len(), 2, "one task per site");
    assert!(p.tick(NOW + 1000).is_empty(), "nothing more while they run");
    let jql = p.jql_for(0, &p.cfg.views[0].clone()).unwrap();
    let fx = p.on_msg(Box::new(JiraMsg::Loaded {
        site: "acme".into(),
        jql,
        action: sync_history::action(&p.cfg.sites[0]),
        result: Ok(sync_history::Synced {
            list: client::TicketsRead {
                tickets: vec![ticket("ABC-30", "New one", "To Do", Category::Todo)],
                ..Default::default()
            },
            ..Default::default()
        }),
    }));
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::PersistCache)),
        "no cache write while beta is still out"
    );
    let jql = p.jql_for(1, &p.cfg.views[0].clone()).unwrap();
    p.on_msg(Box::new(JiraMsg::Loaded {
        site: "beta".into(),
        jql,
        action: sync_history::action(&p.cfg.sites[1]),
        result: Err("Jira turned the email and API token down (401)".into()),
    }));
    let keys: Vec<String> = p.visible().iter().map(|it| it.t.key.clone()).collect();
    assert!(keys.contains(&"ABC-30".to_string()) && !keys.contains(&"ABC-12".to_string()), "{keys:?}");
    assert!(keys.contains(&"XYZ-7".to_string()), "beta's last answer stays");
    assert_eq!(p.sync.errors.len(), 1);
    assert!(screen::status(&p).iter().any(|(s, _)| s == "1 site could not sync"));
}

/// Sample tickets with Jira's numbers, as a search gives them.
fn numbered() -> JiraPlugin {
    let mut p = sample();
    let mut n = 10_000;
    for d in p.data.values_mut() {
        for t in d.tickets.values_mut() {
            n += 1;
            t.id = n.to_string();
        }
    }
    p
}

fn code_link(url: &str) -> Vec<model::CodeLink> {
    vec![model::CodeLink {
        id: "!265".into(),
        title: "Unrelated name".into(),
        url: url.into(),
        status: "open".into(),
        repo: "api".into(),
        branch: "tidy".into(),
    }]
}

#[test]
fn development_links_are_read_in_the_background_a_few_at_a_time() {
    let mut p = numbered();
    let in_view: usize = {
        let v = p.current_view().cloned().unwrap();
        (0..p.cfg.sites.len())
            .filter_map(|i| {
                let jql = p.jql_for(i, &v)?;
                p.data.get(&p.cfg.sites[i].name)?.found.get(&jql).map(Vec::len)
            })
            .sum()
    };
    assert!(in_view > CODE_AT_ONCE, "the sample has more tickets than the bound ({in_view})");
    // The first tick asks for the view and, beside it, a bounded few panels.
    let fx = p.tick(NOW);
    assert_eq!(fx.len(), 2 + CODE_AT_ONCE, "two sites, then the bound");
    assert_eq!(p.sync.code_in_flight.len(), CODE_AT_ONCE);
    assert!(p.sync_code().is_empty(), "no more while that many are out");
    // An answer is held with the ticket's `updated`, and starts the next.
    let id = p.sync.code_in_flight.iter().next().unwrap().clone();
    let (site, key) = id.split_once('\0').unwrap();
    let updated = p.data[site].tickets[key].updated.clone();
    let fx = p.on_msg(Box::new(JiraMsg::Code {
        site: site.into(),
        key: key.into(),
        updated: updated.clone(),
        action: code_history::action(p.cfg.site(site).unwrap(), key),
        result: Ok(client::CodeRead {
            links: code_link("https://gitlab.example/api/-/merge_requests/265"),
            ..Default::default()
        }),
    }));
    assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })), "the next one starts");
    assert_eq!(p.data[site].code[key].links.len(), 1);
    assert_eq!(p.data[site].code[key].updated, updated);
    // The cache carries them.
    let mut q = JiraPlugin::new();
    q.restore(p.snapshot().unwrap());
    assert_eq!(q.data[site].code[key], p.data[site].code[key]);
}

#[test]
fn a_sites_account_id_comes_with_its_first_sync_and_stays_in_the_cache() {
    let mut p = JiraPlugin::new();
    let me = Me { account_id: "712020:abcd".into(), name: "Alice Example".into() };
    p.on_loaded("acme".into(), "q".into(), (Some(me.clone()), vec![]));
    assert_eq!(p.data["acme"].me.as_ref(), Some(&me));
    let mut q = JiraPlugin::new();
    q.restore(p.snapshot().unwrap());
    assert_eq!(q.data["acme"].me.as_ref().map(|m| m.account_id.as_str()), Some("712020:abcd"));
    // A cache from before it was kept reads as no account yet.
    let mut old = JiraPlugin::new();
    old.restore(json!({ "data": { "acme": { "me": { "name": "Alice Example" } } } }));
    assert_eq!(old.data["acme"].me.as_ref().map(|m| m.account_id.as_str()), Some(""));
    assert!(!old.is_mine("acme", &ticket("ABC-1", "x", "To Do", Category::Todo)));
}

#[test]
fn tickets_assigned_to_you_say_you_in_the_tree_the_preview_and_the_page_and_others_do_not() {
    let mut host = hosted();
    let tree = shown(&host, 160, 40);
    let row = |key: &str| tree.lines().find(|l| l.contains(key)).unwrap_or_default().to_string();
    assert!(row("ABC-1 ").contains(" you "), "{tree}");
    assert!(row("DEF-3 ").contains(" you "), "{tree}");
    assert!(row("XYZ-7 ").contains("Alice E."), "{tree}");
    assert!(!row("ABC-13 ").contains(" you "), "{tree}");
    assert!(row("ABC-13 ").contains("Bob"), "{tree}");
    // Its page.
    open_ticket(&mut host, "ABC-12");
    crate::tests::complete_detail(jira_mut(&mut host), "acme", "ABC-12", Ok(detail()));
    let mine = shown(&host, 160, 40);
    let line = mine.lines().find(|l| l.contains("assignee")).unwrap_or_default();
    assert!(line.contains("you") && !line.contains("Alice"), "{mine}");
    // Someone else's.
    let mut host = hosted();
    open_ticket(&mut host, "ABC-13");
    let theirs = shown(&host, 160, 40);
    assert!(theirs.contains("Bob"), "{theirs}");
    assert!(!theirs.contains(" you"), "{theirs}");
    // Before `myself` is known nobody is you.
    let mut p = sample();
    p.data.get_mut("acme").unwrap().me = None;
    assert!(!p.is_mine("acme", &p.data["acme"].tickets["ABC-12"]));
}

#[test]
fn development_links_are_not_asked_again_until_the_ticket_changes() {
    let mut p = numbered();
    p.sync.in_flight.clear();
    // Everything in view is held, as of now.
    let v = p.current_view().cloned().unwrap();
    for i in 0..p.cfg.sites.len() {
        let Some(jql) = p.jql_for(i, &v) else { continue };
        let name = p.cfg.sites[i].name.clone();
        let data = p.data.get_mut(&name).unwrap();
        for k in data.found.get(&jql).cloned().unwrap_or_default() {
            let updated = data.tickets[&k].updated.clone();
            data.code.insert(k, CodeEntry { at: NOW, updated, links: vec![] });
        }
    }
    p.now = NOW + 120;
    assert!(p.sync_code().is_empty(), "held and fresh: nothing asked on a refresh");
    // One ticket changed: only it is asked.
    p.data.get_mut("acme").unwrap().tickets.get_mut("ABC-12").unwrap().updated = "2026-10-07T09:00:00.000+0000".into();
    assert_eq!(p.sync_code().len(), 1);
    assert!(p.sync.code_in_flight.contains(&detail_key("acme", "ABC-12")));
    // A failure is not retried before the refresh interval.
    p.on_msg(Box::new(JiraMsg::Code {
        site: "acme".into(),
        key: "ABC-12".into(),
        updated: String::new(),
        action: code_history::action(p.cfg.site("acme").unwrap(), "ABC-12"),
        result: Err("503".into()),
    }));
    assert!(p.sync_code().is_empty(), "tried a moment ago");
    p.now += p.cfg.refresh_secs as i64;
    assert_eq!(p.sync_code().len(), 1, "tried again after the interval");
    // An unchanged ticket's panel is not read again by the refresh cycle
    // (only ABC-12, whose read failed, is still due) ...
    p.sync.code_in_flight.clear();
    p.now = NOW + CODE_REREAD_SECS - 1;
    assert_eq!(p.sync_code().len(), 1);
    assert!(p.sync.code_in_flight.contains(&detail_key("acme", "ABC-12")));
    // ... but a merge request can be linked without touching the ticket, so
    // half an hour on every open ticket in view is read again (code_reread_tests).
    p.sync.code_in_flight.clear();
    p.now = NOW + CODE_REREAD_SECS;
    assert!(p.sync_code().len() > 1);
}

#[test]
fn a_page_fills_the_development_links_too() {
    let mut p = numbered();
    let mut d = detail();
    d.ticket.id = "10099".into();
    let read = detail_read(&mut p, "acme", "ABC-12");
    p.on_msg(Box::new(JiraMsg::Detail {
        site: "acme".into(), key: "ABC-12".into(),
        action: page_history::action(p.cfg.site("acme").unwrap(), "ABC-12"),
        read,
        result: Ok(Box::new(client::DetailRead { detail: d, ..Default::default() })),
    }));
    assert_eq!(p.data["acme"].code["ABC-12"].links.len(), 1, "the page's read is kept with the rest");
}

#[test]
fn a_merge_request_is_joined_to_a_ticket_by_links_held_in_the_background() {
    let mut p = numbered();
    let url = "https://gitlab.example/api/-/merge_requests/265";
    let mut mr = radar_core::Subject::new(crate::join::MERGE_REQUEST, "api!265", "Unrelated name");
    mr.url = Some(url.into());
    assert!(crate::join::contribute(&p, radar_core::Point::Column, &mr).is_none());
    p.data.get_mut("acme").unwrap().code.insert(
        "ABC-20".into(),
        CodeEntry { at: NOW, updated: String::new(), links: code_link(url) },
    );
    assert!(crate::join::contribute(&p, radar_core::Point::Column, &mr).is_some(), "no page opened");
}

#[test]
fn a_sync_says_which_tickets_changed_and_the_tree_marks_their_rows() {
    let mut p = sample();
    let jql = p.jql_for(0, &p.cfg.views[0].clone()).unwrap();
    let mut held: Vec<Ticket> = p.data["acme"].tickets.values().cloned().collect();
    let story = held.iter_mut().find(|t| t.key == "ABC-12").unwrap();
    story.status = "In Review".into();
    held.push(ticket("ABC-31", "Another one", "To Do", Category::Todo));
    let answer = |p: &mut JiraPlugin, tickets: Vec<Ticket>| {
        p.on_msg(Box::new(JiraMsg::Loaded {
            site: "acme".into(),
            jql: jql.clone(),
            action: sync_history::action(&p.cfg.sites[0]),
            result: Ok(sync_history::Synced {
                list: client::TicketsRead { tickets, ..Default::default() },
                ..Default::default()
            }),
        }))
    };
    let fx = answer(&mut p, held.clone());
    let changes = fx
        .iter()
        .find_map(|e| match e {
            Effect::Changed(c) => Some(c.clone()),
            _ => None,
        })
        .expect("the answer says what changed");
    assert_eq!(changes.changed.iter().collect::<Vec<_>>(), ["ticket:ABC-12"]);
    assert_eq!(changes.added.iter().collect::<Vec<_>>(), ["ticket:ABC-31"]);
    let fx = answer(&mut p, held);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Changed(_))), "the same again");
}

#[test]
fn the_top_bar_says_what_every_part_says() {
    let mut p = sample();
    p.sync.last_ok = Some(p.now - 4);
    p.sync.in_flight.clear();
    assert_eq!(screen::top_right(&p).as_deref(), Some("synced 4s ago · every 2m"));
    p.sync.in_flight.insert("x".into());
    assert_eq!(screen::top_right(&p).as_deref(), Some("syncing Jira · every 2m"));
    let q = JiraPlugin::new();
    assert_eq!(screen::top_right(&q), None, "no site: no sync");
    // The text form has no top bar: the host's chrome view carries the words.
    let words = hosted().chrome_view(160, 40).top_bar.words.map(|badge| badge.text);
    assert_eq!(words.as_deref(), Some("not synced yet · every 2m"));
}

#[test]
fn the_cache_round_trips_and_config_round_trips() {
    let p = sample();
    let mut q = JiraPlugin::new();
    q.restore(p.snapshot().unwrap());
    assert_eq!(q.data, p.data);
    let table = p.config().unwrap();
    q.configure(Some(&table)).unwrap();
    assert_eq!(q.cfg, p.cfg);
    q.configure(None).unwrap();
    assert!(q.cfg.sites.is_empty() && q.cfg.views.len() == 3, "no [jira] table: a working empty plugin");
}

#[test]
fn declares_a_slot_per_site_colour() {
    let names: Vec<String> = JiraPlugin::new().slots().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.len(), config::SITE_COLORS.len());
    assert!(names.contains(&"jira.blue".to_string()));
}

#[test]
fn with_no_site_the_screen_says_what_to_do_and_names_what_it_found() {
    let mut p = JiraPlugin::new();
    p.env = sample().env;
    let mut host = radar_core::Host::new(vec![Box::new(JiraPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    let got = shown(&host, 160, 40);
    // Not set up: the host's set-up page, with what Jira found named in it.
    assert!(got.contains("Jira is not set up yet."), "{got}");
    assert!(got.contains("Add a site: the Jira address, your email and an API token."), "{got}");
    assert!(got.contains("JIRA_OTHER_URL · JIRA_OTHER_EMAIL · JIRA_OTHER_API_TOKEN"), "{got}");
    assert!(!got.contains("other.atlassian.net"), "values are never got");
    let note = {
        let mut q = JiraPlugin::new();
        q.env = sample().env;
        q.first_run().unwrap()
    };
    assert!(note.contains("JIRA_OTHER_URL") && !note.contains("atlassian"), "{note}");
    // A .mcp.json comes first, named by where it is.
    let mut q = JiraPlugin::new();
    q.env = sample().env;
    q.mcp = mcp(MCP_TYPED);
    let note = q.first_run().unwrap();
    assert_eq!(note, "Jira: found tools in /work/acme/.mcp.json; use it under connection");
    assert!(!note.contains("typed-secret-value"));
}

/// Settings › connection with these offers, the given row selected (the
/// offers are first: the .mcp.json's, then the environment's).
fn offer_host(mcp_text: &str, row: usize) -> radar_core::Host {
    let mut p = sample();
    p.mcp = mcp(mcp_text);
    let mut host = radar_core::Host::new(vec![Box::new(JiraPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    section(&mut host, "connection");
    for _ in 0..row {
        press(&mut host, KeyCode::Down);
    }
    host
}

#[test]
fn settings_sites_offers() {
    let host = offer_host(MCP_TYPED, 0);
    let got = shown(&host, 160, 40);
    assert!(!got.contains("typed-secret-value"), "{got}");
    insta::assert_snapshot!(got);
}

#[test]
fn an_mcp_json_site_is_added_reading_its_variables_and_says_where_from() {
    let mut host = offer_host(MCP_REFS, 0);
    let got = shown(&host, 160, 40);
    assert!(got.contains("? Found in /work/acme/.mcp.json: a Jira site (delta.atlassian.net, JIRA_EMAIL, JIRA_TOKEN). Use it?"), "{got}");
    // Where gives way to the names when the row is narrow.
    let narrow = shown(&host, 80, 24);
    assert!(narrow.contains("? Found: a Jira site (delta.atlassian.net, JIRA_EMAIL, JIRA_TOKEN). Use it?"), "{narrow}");
    for name in ["delta.atlassian.net", "JIRA_EMAIL", "JIRA_TOKEN", "\"tools\" entry"] {
        assert!(got.contains(name), "the detail names {name}: {got}");
    }
    assert!(got.contains("acceptance criteria in"), "{got}");
    assert!(got.contains("── detail · found in /work/acme/.mcp.json ──"), "{got}");
    let fx = press(&mut host, KeyCode::Enter);
    assert!(spawned(&fx) >= 1, "checked and fetched at once");
    let s = jira(&host).cfg.sites.last().unwrap().clone();
    assert_eq!(
        (s.name.as_str(), s.url.as_str(), s.email.as_str(), s.token.as_str(), s.from.as_str()),
        ("delta", "https://delta.atlassian.net", "env:JIRA_EMAIL", "env:JIRA_TOKEN", "/work/acme/.mcp.json")
    );
    assert_eq!(s.ac_field("DEL"), Some("customfield_10044"));
    let got = shown(&host, 160, 40);
    assert!(got.contains("from      /work/acme/.mcp.json"), "the site's detail says where: {got}");
    assert!(!got.contains("Found in /work/acme/.mcp.json"), "offered once");
}

#[test]
fn an_mcp_json_token_typed_in_is_left_there_and_its_variable_asked_for() {
    let mut host = offer_host(MCP_TYPED, 0);
    let got = shown(&host, 160, 40);
    assert!(got.contains("Its token is typed into the file: radar does not"), "{got}");
    press(&mut host, KeyCode::Enter);
    let s = jira(&host).cfg.sites.last().unwrap().clone();
    assert_eq!(s.token, "", "never copied");
    assert!(matches!(jira(&host).edit, Some((Target::SiteToken(_), _))), "asks for the env: name");
    assert!(host.message.as_deref().is_some_and(|m| m.contains("does not copy it")), "{:?}", host.message);
    let config = host.config_text();
    assert!(!config.contains("typed-secret-value"), "{config}");
}

// --- ticket pages ----------------------------------------------------------------

fn page_host() -> radar_core::Host {
    let mut host = hosted();
    open_ticket(&mut host, "ABC-12");
    let p = jira_mut(&mut host);
    crate::tests::complete_detail(p, "acme", "ABC-12", Ok(detail()));
    host
}

#[test]
fn enter_opens_the_ticket_page_and_fetches_it() {
    let mut host = hosted();
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    let fx = press(&mut host, KeyCode::Enter);
    assert_eq!(spawned(&fx), 1);
    assert!(host.plugins[0].busy("tickets"));
    let got = shown(&host, 160, 40);
    assert!(got.contains("Jira / acme / ABC / ABC-12"), "{got}");
    assert_eq!(host.plugins[0].panes("tickets"), vec![Pane::Middle]);
    press(&mut host, KeyCode::Esc);
    assert!(jira(&host).pages.is_empty());
    assert!(shown(&host, 160, 40).contains("── detail · ABC-12 ──"), "back on the same row");
}

#[test]
fn ticket_page_overview() {
    insta::assert_snapshot!(shown(&page_host(), 160, 40));
}

#[test]
fn the_screen_and_the_settings_sections_have_icons() {
    use radar_core::ui::icon::Icon;
    let p = sample();
    assert_eq!(p.icon("tickets"), Some(Icon::Issue));
    for s in p.sections() {
        assert!(p.section_icon(s.id).is_some(), "{} has no icon", s.id);
    }
}

#[test]
fn ticket_page_comments() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char(']'));
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn ticket_page_links() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char(']'));
    press(&mut host, KeyCode::Char(']'));
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn ticket_page_code_from_jiras_development_panel() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char('['));
    let got = shown(&host, 160, 40);
    assert!(got.contains("Code>") && got.contains("api !263"), "{got}");
    insta::assert_snapshot!(got);
    // ⏎ shows its merge request: with no part here that owns one, its link.
    let fx = press(&mut host, KeyCode::Enter);
    assert!(
        fx.iter().any(|e| matches!(e, HostEffect::Open(u) if u.ends_with("/merge_requests/263"))),
        "{fx:?}"
    );
}

#[test]
fn links_open_a_linked_tickets_page_and_esc_comes_back() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char(']'));
    press(&mut host, KeyCode::Char(']'));
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    let fx = press(&mut host, KeyCode::Enter);
    assert_eq!(spawned(&fx), 1);
    assert!(shown(&host, 160, 40).contains("Jira / acme / ABC / ABC-15"));
    press(&mut host, KeyCode::Esc);
    assert!(shown(&host, 160, 40).contains("Jira / acme / ABC / ABC-12"));
    // The web link opens in the browser.
    press(&mut host, KeyCode::Down);
    let fx = press(&mut host, KeyCode::Enter);
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Open(u) if u == "https://example.com/design")));
}

#[test]
fn scrolling_to_the_end_of_the_comments_brings_older_ones() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char(']'));
    let _ = shown(&host, 160, 14);
    let got = shown(&host, 160, 14);
    assert!(got.contains("1 older: they load as you scroll down") || got.contains("comments"), "{got}");
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)));
    assert_eq!(spawned(&fx), 1, "the next page of comments");
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)));
    assert_eq!(spawned(&fx), 0, "once");
    let p = jira_mut(&mut host);
    let read = p.details[&detail_key("acme", "ABC-12")].more.unwrap();
    page::on_more(
        p,
        "acme",
        "ABC-12",
        read,
        Ok((
            vec![model::Comment { author: "Carol Example".into(), created: "2026-10-01T08:00:00.000+0000".into(), body: doc("Oldest.") }],
            3,
        )),
    );
    assert!(shown(&host, 160, 40).contains("Oldest."));
}

#[test]
fn an_error_shows_in_the_page() {
    let mut host = hosted();
    open_ticket(&mut host, "ABC-12");
    let p = jira_mut(&mut host);
    crate::tests::complete_detail(p, "acme", "ABC-12", Err("Jira has no ticket ABC-12, or this account cannot see it (404)".into()));
    assert!(shown(&host, 160, 40).contains(":error: could not load: Jira has no ticket ABC-12"));
}

#[test]
fn selecting_a_tab_switches_to_it() {
    let mut host = page_host();
    let got = shown(&host, 160, 40);
    assert!(got.contains("Links"), "{got}");
    host.update(Event::Intent(radar_core::view::Intent::Select { target: radar_core::view::TabView::target(2) }));
    assert_eq!(jira(&host).pages.last().unwrap().tab, 2);
}

// --- the actions menus ----------------------------------------------------------------

#[test]
fn keys_menu() {
    let mut host = hosted();
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    press(&mut host, KeyCode::Char('x'));
    insta::assert_snapshot!(shown(&host, 160, 40));
}

/// Every row of every actions menu (`x`) does something when chosen: on
/// the nav pane, on each tree row and its detail pane, on each tab of a ticket's
/// page and the pages it opens, and on every row of every settings section.
#[test]
fn every_row_and_target_is_taken() {
    use radar_core::check::{Check, Place};
    let tree = || Place::screen("tickets", Pane::Middle);
    let mut places = vec![Place::screen("tickets", Pane::Nav)];
    for down in 0..10 {
        places.push(tree().down(down));
        places.push(tree().down(down).focus(Pane::Detail));
    }
    // Each ticket's page: Overview, Comments, Links, and Code where Jira
    // links a merge request; and a linked ticket's page.
    for down in 2..9 {
        for t in 0..4 {
            places.push(tree().down(down).enter().tab(t));
        }
        places.push(tree().down(down).enter().tab(2).enter());
    }
    let sections: Vec<&'static str> = sample().sections().iter().map(|s| s.id).collect();
    for id in sections {
        // Sites has two offers above the sites.
        for row in 0..18 {
            places.push(Place::section(id).down(row));
        }
    }
    // ABC-12's page with its data, as Jira would give it.
    let make = || {
        let mut p = sample();
        crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(detail()));
        Box::new(p) as Box<dyn Plugin>
    };
    let mut check = Check::new(&make);
    let gaps = check.places(&places);
    assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    assert!(check.checked > 300, "only {} checked", check.checked);
}

/// What Jira adds to a merge request's `x` menu (open its ticket, move it)
/// each returns effects.
#[test]
fn every_action_jira_adds_to_a_merge_request_acts() {
    let make = || Box::new(sample()) as Box<dyn Plugin>;
    let mut check = radar_core::check::Check::new(&make);
    let s = mr("ABC-12: Add a stats endpoint", "feature/def-3-rename", "https://gitlab.example/acme/api/-/merge_requests/263");
    let n = check.actions(&s);
    assert!(n >= 2, "{n} actions");
    assert!(check.gaps.is_empty(), "{:?}", check.gaps);
}

// --- changes to Jira: each asked first, nothing sent in tests ------------------------

fn msg(host: &mut radar_core::Host, m: JiraMsg) -> Vec<HostEffect> {
    let _ = shown(host, 160, 40);
    host.update(Event::Msg { plugin: 0, msg: Box::new(m) })
}

fn ctrl(host: &mut radar_core::Host, c: char) -> Vec<HostEffect> {
    let _ = shown(host, 160, 40);
    host.update(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)))
}

fn moves() -> Vec<model::Transition> {
    let t = |id: &str, name: &str, to: &str, c: Category| model::Transition {
        id: id.into(),
        name: name.into(),
        to: to.into(),
        category: c,
    };
    vec![
        t("11", "Back to do", "To Do", Category::Todo),
        t("31", "Done", "Done", Category::Done),
        t("21", "Start review", "In Review", Category::Doing),
    ]
}

fn menu_text(host: &radar_core::Host) -> Vec<String> {
    host.menu
        .as_ref()
        .map(|m| m.menu.items.iter().map(|i| format!("{} {} {}", i.keys, i.label, i.hint).trim().to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn t_asks_where_a_ticket_can_go_offers_the_usual_next_first_and_asks_before_moving() {
    let mut host = page_host();
    let fx = press(&mut host, KeyCode::Char('t'));
    assert_eq!(spawned(&fx), 1, "Jira is asked for the moves");
    assert!(host.plugins[0].busy("tickets"));
    let read = jira(&host).dialogs.current_read();
    msg(&mut host, JiraMsg::Moves { read, site: "acme".into(), key: "ABC-12".into(), result: Ok(moves()) });
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "move ABC-12 to…");
    assert_eq!(
        menu_text(&host),
        ["now ◐ In Progress", "1 ◐ In Review Jira's “Start review”", "2 ● Done", "3 ○ To Do Jira's “Back to do”"]
    );
    press(&mut host, KeyCode::Char('1'));
    let open = host.menu.as_ref().expect("asked first");
    assert_eq!(open.menu.title, "move ABC-12 to In Review?");
    assert_eq!(menu_text(&host)[0], "◐ In Progress → ◐ In Review (Jira's “Start review”)");
    insta::assert_snapshot!("move_confirm", shown(&host, 160, 40));
    let fx = press(&mut host, KeyCode::Char('n'));
    assert_eq!(spawned(&fx), 0, "no: nothing sent");
    assert_eq!(host.message.as_deref(), Some("nothing sent"));
    // Again, and yes: sent once, got as asked, then Jira's answer.
    press(&mut host, KeyCode::Char('t'));
    let read = jira(&host).dialogs.current_read();
    msg(&mut host, JiraMsg::Moves { read, site: "acme".into(), key: "ABC-12".into(), result: Ok(moves()) });
    press(&mut host, KeyCode::Char('1'));
    let fx = press(&mut host, KeyCode::Char('y'));
    assert_eq!(spawned(&fx), 1);
    assert!(shown(&host, 160, 40).contains(":run_running: asking Jira to move ABC-12 to In Review…"));
    let change = jira(&host).change.clone();
    assert!(change.is_none(), "the question is answered");
    let dispatch = jira(&host).sent[&detail_key("acme", "ABC-12")].dispatch.clone().unwrap();
    let fx = msg(
        &mut host,
        JiraMsg::Wrote {
            dispatch,
            change: write::Change::Move {
                site: "acme".into(),
                key: "ABC-12".into(),
                to: moves()[2].clone(),
                from: "In Progress".into(),
                from_category: Category::Doing,
            },
            result: Ok(serde_json::Value::Null),
        },
    );
    assert!(spawned(&fx) >= 2, "the ticket and the view are fetched again: {fx:?}");
    assert_eq!(host.message.as_deref(), Some("✓ ABC-12 is In Review now"));
    assert!(shown(&host, 160, 40).contains(":run_passed: ABC-12 is In Review now"));
}

#[test]
fn a_long_list_of_moves_is_grouped_with_headings_that_are_never_chosen() {
    let t = |id: &str, to: &str, c: Category| model::Transition { id: id.into(), name: to.into(), to: to.into(), category: c };
    let long = vec![
        t("11", "Backlog", Category::Todo),
        t("31", "Done", Category::Done),
        t("21", "In Review", Category::Doing),
        t("22", "In Test", Category::Doing),
        t("23", "Blocked", Category::Doing),
        t("24", "Ready for release", Category::Doing),
        t("25", "Waiting for design", Category::Doing),
        t("32", "Won't do", Category::Done),
    ];
    let mut host = page_host();
    press(&mut host, KeyCode::Char('t'));
    let read = jira(&host).dialogs.current_read();
    msg(&mut host, JiraMsg::Moves { read, site: "acme".into(), key: "ABC-12".into(), result: Ok(long) });
    let got = menu_text(&host);
    assert_eq!(got[1], "usual next");
    assert!(got.iter().any(|l| l == "◐ in progress · 2") && got.iter().any(|l| l == "● done · 2"), "{got:?}");
    insta::assert_snapshot!("move_menu_grouped", shown(&host, 160, 40));
    // It opens on the usual next move; Down moves over headings to the
    // next move, and ⏎ asks about it.
    assert_eq!(host.menu.as_ref().unwrap().row, 2, "{got:?}");
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Enter);
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "move ABC-12 to Backlog?");
}

#[test]
fn a_dispatched_change_error_is_said_as_unconfirmed_on_the_page() {
    let mut host = page_host();
    let change = write::Change::Move {
        site: "acme".into(),
        key: "ABC-12".into(),
        to: moves()[1].clone(),
        from: "In Progress".into(),
        from_category: Category::Doing,
    };
    radar_core::writes::set_mode(radar_core::writes::Mode::Edit, None);
    let _ = write::send(jira_mut(&mut host), change.clone());
    let dispatch = jira(&host).sent[&detail_key("acme", "ABC-12")].dispatch.clone().unwrap();
    let fx = msg(
        &mut host,
        JiraMsg::Wrote { dispatch, change, result: Err("Jira refused the request (400): Resolution is required.".into()) },
    );
    assert_eq!(spawned(&fx), 0, "never tried again");
    let got = shown(&host, 160, 40);
    assert!(got.contains(":error: Jira did not confirm move ABC-12 to Done"), "{got}");
    let notice = host.message.as_deref().unwrap();
    assert!(notice.contains("Jira refused the request (400): Resolution is required."), "{notice}");
    assert!(notice.contains("check Jira before trying again"), "{notice}");
}

#[test]
fn c_composes_a_comment_shows_exactly_what_will_post_and_posts_once() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char('c'));
    assert!(host.plugins[0].capturing());
    typed(&mut host, "Fixed in **!263**, see https://example.com/x");
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Enter);
    typed(&mut host, "- tests");
    insta::assert_snapshot!("comment_composer", shown(&host, 160, 40));
    // q and x are text here, not the host's.
    typed(&mut host, " qx");
    assert!(!host.quit && host.menu.is_none());
    ctrl(&mut host, 's');
    let open = host.menu.as_ref().expect("asked first");
    assert_eq!(open.menu.title, "post this on ABC-12?");
    assert_eq!(
        menu_text(&host),
        [
            "Fixed in !263, see https://example.com/x",
            "",
            "• tests qx",
            "y yes yes, post it",
            "e edit keep writing",
            "n no no, drop it"
        ]
    );
    insta::assert_snapshot!("comment_confirm", shown(&host, 160, 40));
    // e: back to writing, nothing lost.
    press(&mut host, KeyCode::Char('e'));
    assert!(host.plugins[0].capturing());
    ctrl(&mut host, 's');
    let fx = press(&mut host, KeyCode::Char('y'));
    assert_eq!(spawned(&fx), 1, "sent once");
    assert!(!host.plugins[0].capturing(), "the composer is closed");
    // Jira refused: the text is kept for c, and nothing is sent again.
    let dispatch = jira(&host).sent[&detail_key("acme", "ABC-12")].dispatch.clone().unwrap();
    let fx = msg(
        &mut host,
        JiraMsg::Wrote {
            dispatch,
            change: write::Change::Comment {
                site: "acme".into(),
                key: "ABC-12".into(),
                text: "Fixed in **!263**, see https://example.com/x\n\n- tests qx".into(),
            },
            result: Err("Jira answered 503".into()),
        },
    );
    assert_eq!(spawned(&fx), 0);
    assert!(
        host.message
            .as_deref()
            .is_some_and(|m| m.ends_with("original text kept · ctrl-r")),
        "{:?}",
        host.message
    );
    press(&mut host, KeyCode::Char('c'));
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Char('e'));
    assert_eq!(
        jira(&host).compose.as_ref().unwrap().text(),
        "Fixed in **!263**, see https://example.com/x\n\n- tests qx"
    );
}

#[test]
fn a_declined_comment_is_kept_for_c() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char('c'));
    typed(&mut host, "not yet");
    ctrl(&mut host, 's');
    let fx = press(&mut host, KeyCode::Char('n'));
    assert_eq!(spawned(&fx), 0);
    assert_eq!(
        host.message.as_deref(),
        Some("nothing sent; original text kept · ctrl-r")
    );
    assert!(!host.plugins[0].capturing(), "the composer is closed");
    press(&mut host, KeyCode::Char('c'));
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Char('e'));
    assert_eq!(jira(&host).compose.as_ref().unwrap().text(), "not yet");
}

#[test]
fn each_mode_sends_only_the_writes_it_allows_and_its_question_says_so() {
    use radar_core::writes::Mode;
    for mode in Mode::ALL {
        let mut host = page_host();
        host.set_env_mode(Some((mode, "RADAR_MODE".into())));
        // A comment adds: write and edit send it.
        press(&mut host, KeyCode::Char('c'));
        typed(&mut host, "hello");
        ctrl(&mut host, 's');
        let got = menu_text(&host);
        if mode == Mode::ReadOnly {
            assert_eq!(
                got,
                [
                    "hello",
                    "radar is read only (set by RADAR_MODE): nothing will be sent",
                    "y yes yes, post it",
                    "e edit keep writing",
                    "n no no, drop it",
                ]
            );
        } else {
            assert_eq!(got.len(), 4, "{mode:?}: no note: {got:?}");
        }
        let fx = press(&mut host, KeyCode::Char('y'));
        assert_eq!(spawned(&fx), usize::from(mode != Mode::ReadOnly), "{mode:?}: comment");
        if mode == Mode::ReadOnly {
            assert_eq!(
                host.message.as_deref(),
                Some(
                    "✗ Jira did not comment on ABC-12: not sent: radar is read only; original text kept · ctrl-r"
                )
            );
            press(&mut host, KeyCode::Char('c'));
            press(&mut host, KeyCode::Enter);
            press(&mut host, KeyCode::Char('e'));
            assert_eq!(
                jira(&host).compose.as_ref().unwrap().text(),
                "hello",
                "kept"
            );
            press(&mut host, KeyCode::Esc);
            press(&mut host, KeyCode::Char('y'));
        }
        // A move changes what is there: only edit sends it.
        let mut host = page_host();
        host.set_env_mode(Some((mode, "RADAR_MODE".into())));
        // The x menu lists it, muted with why while the mode refuses it.
        press(&mut host, KeyCode::Char('x'));
        let row = menu_text(&host).into_iter().find(|l| l.contains("another status")).unwrap();
        match mode {
            Mode::ReadOnly => assert!(row.ends_with("· radar is read only"), "{row}"),
            Mode::Write => assert!(row.ends_with("· needs edit mode"), "{row}"),
            Mode::Edit => assert!(row.ends_with("another status…"), "{row}"),
        }
        let comment = menu_text(&host).into_iter().find(|l| l.contains("comment on")).unwrap();
        assert_eq!(comment.contains("·"), mode == Mode::ReadOnly, "{mode:?}: {comment}");
        press(&mut host, KeyCode::Esc);
        press(&mut host, KeyCode::Char('t'));
        let read = jira(&host).dialogs.current_read();
        msg(&mut host, JiraMsg::Moves { read, site: "acme".into(), key: "ABC-12".into(), result: Ok(moves()) });
        press(&mut host, KeyCode::Char('1'));
        let note = menu_text(&host)[1].clone();
        match mode {
            Mode::ReadOnly => assert_eq!(note, "radar is read only (set by RADAR_MODE): nothing will be sent"),
            Mode::Write => assert_eq!(note, "radar is in write mode (set by RADAR_MODE): this needs edit mode, nothing will be sent"),
            Mode::Edit => assert_eq!(note, "y yes yes, move it"),
        }
        let fx = press(&mut host, KeyCode::Char('y'));
        assert_eq!(spawned(&fx), usize::from(mode == Mode::Edit), "{mode:?}: move");
        if mode != Mode::Edit {
            assert!(
                host.message.as_deref().is_some_and(|m| m.starts_with("✗ Jira did not move ABC-12 to In Review: not sent")),
                "{:?}",
                host.message
            );
        }
    }
}

/// Work started on ABC-12 elsewhere: Jira's answers arrive, one question
/// asks, and each mode sends only what it allows.
#[test]
fn starting_work_asks_once_and_the_mode_decides_what_y_sends() {
    use radar_core::writes::Mode;
    for mode in Mode::ALL {
        let mut host = hosted();
        host.set_env_mode(Some((mode, "RADAR_MODE".into())));
        {
            let t = jira_mut(&mut host).data.get_mut("acme").unwrap().tickets.get_mut("ABC-12").unwrap();
            t.status = "To Do".into();
            t.category = Category::Todo;
            t.assignee = "Bob Example".into();
            t.assignee_id = "acc-bob".into();
        }
        let me = model::Me { account_id: "acc-alice".into(), name: "Alice Example".into() };
        msg(
            &mut host,
            JiraMsg::Started { site: "acme".into(), key: "ABC-12".into(), me: Some(Ok(me)), moves: Some(Ok(moves())) },
        );
        let got = menu_text(&host);
        assert_eq!(host.menu.as_ref().unwrap().menu.title, "start ABC-12 on Jira?");
        assert_eq!(got[0], "assignee: Bob Example → you (Alice Example)", "{mode:?}");
        assert_eq!(got[1], "status: To Do → In Review", "{mode:?}: the first move into progress");
        let fx = press(&mut host, KeyCode::Char('y'));
        match mode {
            Mode::Edit => assert_eq!(spawned(&fx), 1, "the assignment goes first, the move after its answer"),
            _ => {
                assert_eq!(spawned(&fx), 0, "{mode:?}");
                assert!(
                    host.message.as_deref().is_some_and(|m| m.starts_with("✗ Jira did not assign ABC-12 to you: not sent")),
                    "{:?}",
                    host.message
                );
                assert!(jira(&host).start_next.is_none(), "no move left waiting");
            }
        }
    }
}

#[test]
fn esc_in_the_composer_asks_before_dropping_text() {
    let mut host = page_host();
    press(&mut host, KeyCode::Char('c'));
    press(&mut host, KeyCode::Esc);
    assert!(!host.plugins[0].capturing(), "nothing typed: closed at once");
    press(&mut host, KeyCode::Char('c'));
    typed(&mut host, "half a thought");
    press(&mut host, KeyCode::Esc);
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "stop: drop the comment?");
    press(&mut host, KeyCode::Char('n'));
    assert_eq!(jira(&host).compose.as_ref().unwrap().text(), "half a thought");
    press(&mut host, KeyCode::Esc);
    press(&mut host, KeyCode::Char('y'));
    assert!(jira(&host).compose.is_none());
    // An empty comment is never sent.
    press(&mut host, KeyCode::Char('c'));
    let fx = ctrl(&mut host, 's');
    assert_eq!(spawned(&fx), 0);
    assert!(host.menu.is_none());
}

#[test]
fn n_makes_a_new_ticket_from_project_type_summary_and_description() {
    let mut host = hosted();
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    let fx = press(&mut host, KeyCode::Char('n'));
    assert_eq!(spawned(&fx), 1, "ABC's types are asked, ABC being the ticket's project");
    let types = vec![
        model::IssueType { id: "10001".into(), name: "Story".into(), subtask: false },
        model::IssueType { id: "10004".into(), name: "Bug".into(), subtask: false },
    ];
    let read = jira(&host).dialogs.current_read();
    msg(&mut host, JiraMsg::Types { read, site: "acme".into(), project: "ABC".into(), result: Ok(types) });
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "a new ticket in ABC: which type?");
    press(&mut host, KeyCode::Char('2'));
    assert!(host.plugins[0].capturing());
    press(&mut host, KeyCode::Enter);
    assert!(host.message.as_deref().is_some_and(|m| m.starts_with("not yet")), "a summary is needed");
    typed(&mut host, "Stats count twice");
    press(&mut host, KeyCode::Enter);
    typed(&mut host, "Seen on the dashboard.");
    insta::assert_snapshot!("new_ticket_description", shown(&host, 160, 40));
    ctrl(&mut host, 's');
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "create this in ABC on acme?");
    assert_eq!(
        menu_text(&host),
        ["Bug · Stats count twice", "", "Seen on the dashboard.", "y yes yes, create it", "e edit keep writing", "n no no, drop it"]
    );
    // Declined, it is kept for N: the summary, then the description.
    press(&mut host, KeyCode::Char('n'));
    assert_eq!(
        host.message.as_deref(),
        Some("nothing sent; original text kept · ctrl-r")
    );
    press(&mut host, KeyCode::Char('n'));
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Char('e'));
    assert_eq!(
        jira(&host).compose.as_ref().unwrap().text(),
        "Stats count twice"
    );
    press(&mut host, KeyCode::Enter);
    assert_eq!(
        jira(&host).compose.as_ref().unwrap().text(),
        "Seen on the dashboard."
    );
    ctrl(&mut host, 's');
    let fx = press(&mut host, KeyCode::Char('y'));
    assert_eq!(spawned(&fx), 1);
    let change = write::Change::Create {
        site: "acme".into(),
        project: "ABC".into(),
        kind: model::IssueType { id: "10004".into(), name: "Bug".into(), subtask: false },
        summary: "Stats count twice".into(),
        description: "Seen on the dashboard.".into(),
    };
    let req = change.request().unwrap();
    assert_eq!(req.body.as_ref().unwrap()["fields"]["issuetype"]["id"], "10004");
    let dispatch = jira(&host).sent[&detail_key("acme", "ABC")].dispatch.clone().unwrap();
    let fx = msg(&mut host, JiraMsg::Wrote { dispatch, change, result: Ok(serde_json::json!({ "id": "10300", "key": "ABC-31" })) });
    assert!(spawned(&fx) >= 1);
    assert_eq!(host.message.as_deref(), Some("✓ created ABC-31"));
    assert!(shown(&host, 160, 40).contains("Jira / acme / ABC / ABC-31"), "its page opens");
}

#[test]
fn n_with_several_watched_projects_asks_which_first() {
    let mut host = hosted();
    press(&mut host, KeyCode::Char('n'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "a new ticket in which project?");
    assert_eq!(menu_text(&host), ["1 ABC on acme", "2 DEF on acme"]);
    let fx = press(&mut host, KeyCode::Char('2'));
    assert_eq!(spawned(&fx), 1, "DEF's types");
}

#[test]
fn i_assigns_to_you_after_asking_and_says_when_it_is_yours_already() {
    let mut host = hosted();
    // ABC-12 is Alice's, and Alice is who acme's token is.
    for _ in 0..3 {
        press(&mut host, KeyCode::Down);
    }
    press(&mut host, KeyCode::Char('i'));
    assert_eq!(host.message.as_deref(), Some("ABC-12 is already yours"));
    assert!(host.menu.is_none());
    // ABC-20 is nobody's.
    let mut host = hosted();
    open_ticket(&mut host, "ABC-20");
    press(&mut host, KeyCode::Char('i'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "assign ABC-20 to you?");
    assert_eq!(menu_text(&host)[0], "unassigned → Alice Example (you)");
    let fx = press(&mut host, KeyCode::Char('y'));
    assert_eq!(spawned(&fx), 1);
    // On beta, who you are is asked first.
    let mut host = hosted();
    open_ticket(&mut host, "XYZ-7");
    let fx = press(&mut host, KeyCode::Char('i'));
    assert_eq!(spawned(&fx), 1);
    assert!(host.menu.is_none());
    let read = jira(&host).dialogs.current_read();
    msg(&mut host, JiraMsg::Who { read, site: "beta".into(), key: "XYZ-7".into(), result: Ok(Me { account_id: "acc-a2".into(), name: "Alice Example".into() }) });
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "assign XYZ-7 to you?");
    assert_eq!(menu_text(&host)[0], "Alice Example → Alice Example (you)");
}

// --- joining merge requests ---------------------------------------------------------------

fn mr(title: &str, branch: &str, url: &str) -> radar_core::Subject {
    radar_core::Subject::new("merge-request", "acme/team-a/api!263", "api !263")
        .with_url(url)
        .with("title", title)
        .with("branch", branch)
}

#[test]
fn a_merge_request_naming_a_ticket_gets_its_column_section_tab_and_actions() {
    use radar_core::{Contribution, Point};
    let p = sample();
    let s = mr("ABC-12: Add a stats endpoint", "feature/def-3-rename", "https://gitlab.example/acme/api/-/merge_requests/263");
    let related = p.related(&s);
    assert_eq!(related.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["ABC-12", "DEF-3"]);
    assert!(related[0].url.as_deref().unwrap().ends_with("/browse/ABC-12"));
    // A ticket column, not chips: the first ticket, how many more, and a
    // click on it opens that ticket.
    let Some(Contribution::Cell(cell)) = p.contribute(Point::Column, &s) else { panic!() };
    assert_eq!(
        (cell.heading.as_str(), cell.text.as_str(), cell.tone),
        ("ticket", "ABC-12 · In Progress +1", radar_core::Tone::Info)
    );
    let got = cell.show.expect("it opens the ticket");
    assert_eq!((got.kind.as_str(), got.id.as_str()), ("ticket", "ABC-12"));
    let Some(Contribution::Section { icon, title, lines }) = p.contribute(Point::Section, &s) else { panic!() };
    assert_eq!(icon, Some(radar_core::ui::icon::Icon::Issue), "marked like the owner's sections");
    assert_eq!(title, "Tickets");
    assert_eq!(lines[0].to_string(), "ABC-12 · In Progress · Story");
    let Some(Contribution::Actions(actions)) = p.contribute(Point::Actions, &s) else { panic!() };
    let rows: Vec<(Option<char>, &str)> = actions.iter().map(|a| (a.key, a.what.as_str())).collect();
    assert_eq!(
        rows,
        [(Some('v'), "view ABC-12"), (Some('>'), "move ABC-12 to …"), (None, "view DEF-3"), (None, "move DEF-3 to …")]
    );
    // A Tickets tab on its page: a row per ticket, ⏎ opens the ticket.
    let Some(Contribution::Tab { title, rows }) = p.contribute(Point::Tab, &s) else { panic!() };
    assert_eq!(title, "Tickets");
    let lines: Vec<String> = rows.iter().map(|r| r.line.to_string()).collect();
    assert_eq!(lines[0], "ABC-12 · In Progress · Story · Add a stats endpoint for the dashboard");
    assert!(lines[1].starts_with("DEF-3 · "), "{lines:?}");
    let opens = rows[0].subject.as_ref().expect("the row opens the ticket");
    assert_eq!((opens.kind.as_str(), opens.id.as_str()), ("ticket", "ABC-12"));
    assert!(p.contribute(Point::Chips, &s).is_none(), "a column, not chips");
    // Nothing for one that names no ticket, or for other kinds.
    assert!(p.contribute(Point::Column, &mr("Tidy the build", "tidy", "https://x/1")).is_none());
    let ticket = radar_core::Subject::new("ticket", "ABC-12", "ABC-12").with("title", "ABC-12");
    assert!(p.contribute(Point::Chips, &ticket).is_none());
}

#[test]
fn a_clones_branch_naming_a_ticket_gets_only_its_ticket_column() {
    use radar_core::{Contribution, Point};
    let p = sample();
    let branch = radar_core::Subject::new("checkout", "/work/acme/api", "api")
        .with("repo", "acme/team-a/api")
        .with("branch", "alice/abc-12-stats");
    let Some(Contribution::Cell(cell)) = p.contribute(Point::Column, &branch) else { panic!() };
    assert_eq!((cell.heading.as_str(), cell.text.as_str()), ("ticket", "ABC-12 · In Progress"));
    assert!(matches!(&cell.show, Some(t) if t.kind == "ticket" && t.id == "ABC-12"), "a click opens it");
    for point in [Point::Section, Point::Tab, Point::Actions, Point::Chips] {
        assert!(p.contribute(point, &branch).is_none(), "{point:?}");
    }
    let other = branch.clone().with("branch", "spike");
    assert!(p.contribute(Point::Column, &other).is_none(), "no ticket named");
}

#[test]
fn jiras_development_panel_joins_a_merge_request_whose_title_names_no_ticket() {
    use radar_core::{Contribution, Point};
    let mut p = sample();
    let url = "https://gitlab.com/acme/team-a/api/-/merge_requests/263";
    let s = mr("Tidy the stats", "tidy", url);
    assert!(p.contribute(Point::Column, &s).is_none(), "not linked yet");
    crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(detail()));
    let Some(Contribution::Cell(cell)) = p.contribute(Point::Column, &s) else { panic!() };
    assert_eq!(cell.text, "ABC-12 · In Progress");
    assert_eq!(p.related(&s)[0].id, "ABC-12");
    // Its actions act on that ticket.
    let fx = p.act(&s, "open\0ABC-12");
    assert!(matches!(&fx[..], [Effect::Show(t)] if t.kind == "ticket" && t.id == "ABC-12" && t.field("site") == Some("acme")), "{fx:?}");
    let fx = p.act(&s, "move\0ABC-12");
    assert!(fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. })), "the moves are asked");
}

#[test]
fn showing_a_ticket_opens_its_page_on_its_site_and_esc_goes_back() {
    let mut p = sample();
    let s = radar_core::Subject::new("ticket", "XYZ-7", "XYZ-7");
    let radar_core::Showing::Page { screen, effects } = p.show(&s) else { panic!("a page") };
    assert_eq!(screen, "tickets");
    assert!(effects.iter().any(|e| matches!(e, Effect::SpawnAction { .. })));
    assert_eq!((p.pages[0].site.as_str(), p.depth("tickets")), ("beta", 1));
    assert!(matches!(p.show(&radar_core::Subject::new("ticket", "not a key", "x")), radar_core::Showing::Cannot));
    assert!(matches!(p.show(&radar_core::Subject::new("thread", "C1/1", "x")), radar_core::Showing::Cannot));
    // What the page is about, for the actions others add.
    let about = p.subject("tickets", Pane::Middle).unwrap();
    assert_eq!((about.kind.as_str(), about.id.as_str()), ("ticket", "XYZ-7"));
    assert_eq!(about.url.as_deref(), Some("https://beta.atlassian.net/browse/XYZ-7"));
}

// --- settings ---------------------------------------------------------------------

#[test]
fn settings_sites() {
    let mut host = hosted();
    section(&mut host, "connection");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn settings_sites_selected_site() {
    let mut host = hosted();
    section(&mut host, "connection");
    // Past the two found and the add row, acme: its address.
    for _ in 0..4 {
        press(&mut host, KeyCode::Down);
    }
    insta::assert_snapshot!(shown(&host, 160, 40));
}

/// Each site reads as every part's connection does: where, the account and
/// token, who they sign in as, the check; ⏎ on the check asks Jira.
#[test]
fn a_sites_rows_are_in_the_order_every_part_has_and_its_check_asks_jira() {
    let mut host = hosted();
    section(&mut host, "connection");
    let got = shown(&host, 160, 40);
    let at = |label: &str| got.find(&format!("  {label}  ")).unwrap_or_else(|| panic!("{label}: {got}"));
    let order = ["site", "account", "token", "signed in as", "check"].map(at);
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{got}");
    assert!(got.find("  check  ").unwrap() < got.find("\n  refresh ").unwrap());
    assert!(!got.contains("pause if idle"), "the host's now");
    // Past the two found, the add row, acme and five of its rows: its check.
    for _ in 0..8 {
        press(&mut host, KeyCode::Down);
    }
    let fx = press(&mut host, KeyCode::Enter);
    assert_eq!(spawned(&fx), 1, "asks Jira");
    assert!(shown(&host, 160, 40).contains("asking Jira…"));
}

#[test]
fn settings_projects() {
    let mut host = hosted();
    section(&mut host, "projects");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn projects_section_uses_neutral_tree_with_selectable_site_rows() {
    let mut p = sample();
    let view = settings::projects_view(&p, 100, p.now);
    let radar_core::view::Node::Tree(tree) = &view.body else { panic!("projects is a tree") };
    assert!(!tree.rows[0].heading);
    assert_eq!(tree.selected, ["s:acme", "add"]);
    assert!(tree.open.contains(&vec!["s:acme".into()]));
    let down = radar_core::view_tui::section_key(&view, radar_core::view::Chord::new(radar_core::view::Key::Down));
    settings::projects_intent(&mut p, &down);
    assert!(matches!(settings::project_rows(&p)[p.section_rows[1]], settings::ProjRow::Colour(0)));
    let wire = serde_json::to_string(&view).unwrap();
    assert!(wire.contains("\"scope\":\"projects\""));
}

#[test]
fn home_end_and_pages_in_projects_keep_a_selected_visible_tree_row() {
    let mut host = hosted();
    section(&mut host, "projects");
    for k in [KeyCode::End, KeyCode::Home, KeyCode::PageDown, KeyCode::PageUp, KeyCode::Down, KeyCode::Up] {
        press(&mut host, k);
        let p = jira(&host);
        let radar_core::view::Node::Tree(tree) = settings::projects_view(p, 100, p.now).body else { panic!("projects tree") };
        assert!(tree.visible().iter().any(|(path, _)| *path == tree.selected), "{k:?}");
    }
    press(&mut host, KeyCode::Home);
    let p = jira(&host);
    assert!(matches!(crate::settings::project_rows(p)[p.section_rows[1]], crate::settings::ProjRow::Site(0)));
}

#[test]
fn settings_views() {
    let mut host = hosted();
    section(&mut host, "views");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn views_section_keeps_editing_and_actions_in_neutral_view() {
    let mut p = sample();
    let view = settings::views_view(&p, 80);
    let radar_core::view::Node::List { rows, selected, editing } = &view.body else { panic!("views is a list") };
    assert_eq!(*selected, Some(0));
    assert_eq!(rows[0].target.as_str(), "row:0");
    assert!(editing.is_none());
    assert!(serde_json::to_string(&view).unwrap().contains("jira.views.kept_text"));
    settings::views_intent(&mut p, &radar_core::view::Intent::Activate { target: radar_core::view::Target::new("row:0") });
    let editing_view = settings::views_view(&p, 80);
    let radar_core::view::Node::List { editing: Some(editing), .. } = editing_view.body else { panic!("editor in view") };
    assert_eq!((editing.row, editing.at), (0, 4));
}

#[test]
fn connection_section_exposes_rows_and_detail_as_neutral_data() {
    let mut p = sample();
    let view = settings::connection_view(&p, 100, p.now);
    let radar_core::view::Node::Tree(tree) = &view.body else { panic!("connection is a tree") };
    let at = p.section_rows[0];
    assert_eq!(tree.selected, vec![format!("row:{at}")]);
    assert_eq!(tree.visible().len(), settings::site_rows(&p).len(), "a site's fields sit under it");
    assert!(!view.detail.is_empty());
    let next = radar_core::view_tui::section_key(&view, radar_core::view::Chord::new(radar_core::view::Key::Down));
    settings::connection_intent(&mut p, &next);
    assert_eq!(p.section_rows[0], 1);
    assert!(serde_json::to_string(&view).unwrap().contains("\"scope\":\"connection\""));
}

#[test]
fn settings_look() {
    let mut host = hosted();
    section(&mut host, "look");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn look_section_is_renderer_neutral_and_skips_group_headings() {
    let mut p = sample();
    let view = settings::look_view(&p);
    let radar_core::view::Node::List { rows, selected, .. } = &view.body else { panic!("look is a list") };
    assert_eq!(*selected, Some(1));
    assert!(rows[0].heading);
    assert_eq!(rows[1].target.as_str(), "row:1");
    let wire = serde_json::to_string(&view).unwrap();
    assert!(wire.contains("\"scope\":\"look\""));
    assert!(wire.contains("jira.look.kept_text"), "recovery remains reachable from neutral actions");
    let down = radar_core::view_tui::section_key(&view, radar_core::view::Chord::new(radar_core::view::Key::Down));
    assert_eq!(down, radar_core::view::Intent::Select { target: radar_core::view::Target::new("row:2") });
    settings::look_intent(&mut p, &down);
    assert_eq!(p.section_rows[3], 2);
}

#[test]
fn an_offer_from_the_environment_adds_a_site_that_reads_the_variables() {
    let mut host = hosted();
    section(&mut host, "connection");
    // Below the .mcp.json's offer.
    press(&mut host, KeyCode::Down);
    let got = shown(&host, 160, 40);
    assert!(got.contains("? Found in your environment: a Jira site (JIRA_OTHER_URL, JIRA_OTHER_EMAIL, JIRA_OTHER_API_TOKEN). Use it?"), "{got}");
    assert!(got.contains("── detail · found in your environment ──"), "{got}");
    for name in ["JIRA_OTHER_URL", "JIRA_OTHER_EMAIL", "JIRA_OTHER_API_TOKEN"] {
        assert!(got.contains(name), "the detail names {name}: {got}");
    }
    assert!(got.contains("shows the values or saves them"), "{got}");
    assert!(!got.contains("other.atlassian.net") && !got.contains("not-a-real-token"), "{got}");
    let fx = press(&mut host, KeyCode::Enter);
    assert!(spawned(&fx) >= 1, "checked at once");
    let s = jira(&host).cfg.sites.last().unwrap().clone();
    assert_eq!((s.name.as_str(), s.url.as_str(), s.token.as_str()), ("other", "env:JIRA_OTHER_URL", "env:JIRA_OTHER_API_TOKEN"));
    assert!(!shown(&host, 160, 40).contains("JIRA_OTHER_URL, JIRA_OTHER_EMAIL"), "offered once");
}

#[test]
fn adding_a_site_by_typing_validates_and_moves_on_to_its_email() {
    let mut host = hosted();
    section(&mut host, "connection");
    // Past what was found (a .mcp.json's site, the environment's): the add row.
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing());
    typed(&mut host, "gamma");
    let got = shown(&host, 160, 40);
    assert!(got.contains(":error: not a web address"), "{got}");
    press(&mut host, KeyCode::Enter);
    assert!(host.message.as_deref().is_some_and(|m| m.starts_with("not yet:")), "{:?}", host.message);
    for _ in 0..5 {
        press(&mut host, KeyCode::Backspace);
    }
    host.update(Event::Paste("https://gamma.atlassian.net/jira/software/projects/GAM/boards/2\n".into()));
    assert!(shown(&host, 160, 40).contains(":run_passed: the site gamma.atlassian.net"));
    press(&mut host, KeyCode::Enter);
    let p = jira(&host);
    assert_eq!(p.cfg.sites.last().unwrap().url, "https://gamma.atlassian.net");
    assert!(matches!(p.edit, Some((Target::SiteEmail(2), _))), "its email next");
    let s = p.cfg.sites.last().unwrap();
    assert_eq!((s.email.as_str(), s.token.as_str()), ("env:JIRA_EMAIL", "env:JIRA_TOKEN"), "the team's names");
    assert!(shown(&host, 160, 40).contains("env:JIRA_EMAIL"), "the input starts from it");
    host.update(Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    typed(&mut host, "alice@example.com");
    press(&mut host, KeyCode::Enter);
    let open = host.menu.as_ref().expect("asks to check");
    assert_eq!(open.menu.title, "check gamma now?");
    let fx = press(&mut host, KeyCode::Char('y'));
    assert_eq!(spawned(&fx), 1);
}

#[test]
fn a_typed_token_is_masked_and_an_env_name_must_be_set() {
    let mut p = sample();
    p.cfg.sites[0].token = "ATATT3xFfGF0-not-a-real-token-1234".into();
    assert_eq!(field_value_for_test(&p), "ATAT…1234 (typed in)");
    let t = Target::SiteToken(0);
    let v = |s: &str| settings_validate(&p, &t, s);
    assert!(v("env:RADAR_JIRA_SURELY_UNSET_VAR").unwrap_err().contains("is not set"));
    assert!(v("short").unwrap_err().contains("too short"));
    assert!(v("a-long-enough-token-value").is_ok());
}

fn field_value_for_test(p: &JiraPlugin) -> String {
    let mut host = radar_core::Host::new(vec![Box::new(JiraPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    let mut q = sample();
    q.cfg = p.cfg.clone();
    host.plugins[0] = Box::new(q);
    section(&mut host, "connection");
    let got = shown(&host, 160, 40);
    got
        .lines()
        .find(|l| l.trim_start_matches(['>', ' ']).starts_with("token "))
        .and_then(|l| l.trim_start_matches(['>', ' ']).strip_prefix("token"))
        .and_then(|s| s.split("  {row:").next())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn settings_validate(p: &JiraPlugin, t: &Target, s: &str) -> Result<String, String> {
    // Through the detail panel's verdict, as the user sees it.
    let mut q = sample();
    q.cfg = p.cfg.clone();
    q.edit = Some((t.clone(), radar_core::input::TextInput::new(s)));
    // On the row being typed into.
    let token = settings::SiteRow::Field(0, settings::Field::Token);
    q.section_rows[0] = settings::site_rows(&q).iter().position(|r| *r == token).unwrap();
    let mut host = radar_core::Host::new(vec![Box::new(JiraPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(q);
    section(&mut host, "connection");
    let got = shown(&host, 160, 40);
    let verdict = got
        .lines()
        .skip_while(|l| !l.contains("AS YOU TYPE"))
        .nth(1)
        .unwrap_or_default()
        .trim()
        .to_string();
    match verdict.strip_prefix(":run_passed: ") {
        Some(ok) => Ok(ok.into()),
        None => Err(verdict),
    }
}

#[test]
fn watching_a_project_from_a_pasted_link_offers_a_check() {
    let mut host = hosted();
    section(&mut host, "projects");
    // acme's heading is skipped; its add row first.
    press(&mut host, KeyCode::Enter);
    assert!(host.plugins[0].capturing());
    host.update(Event::Paste("https://acme.atlassian.net/browse/OPS-4".into()));
    assert!(shown(&host, 160, 40).contains(":run_passed: will watch OPS on acme"));
    let fx = press(&mut host, KeyCode::Enter);
    assert!(spawned(&fx) >= 1, "the view refreshes with it");
    assert!(jira(&host).cfg.sites[0].watches("OPS"));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "check OPS on acme now?");
}

#[test]
fn the_acceptance_criteria_field_is_picked_among_fields_named_like_it() {
    let mut p = sample();
    assert!(settings::ac_like("Acceptance Criteria") && settings::ac_like("AC") && !settings::ac_like("Account"));
    let fx = settings::on_ac_fields(
        &mut p,
        "acme",
        "DEF",
        Ok(vec![
            ("customfield_10050".into(), "Acceptance Criteria".into(), 37),
            ("customfield_10090".into(), "Acceptance criteria (old)".into(), 2),
        ]),
    );
    let Effect::Menu(m) = &fx[0] else { panic!("{fx:?}") };
    assert_eq!(m.title, "which field holds DEF's acceptance criteria?");
    assert_eq!(m.items[0].hint, "customfield_10050 · 37 of DEF's tickets fill it in");
    assert_eq!(m.items.last().unwrap().label, "none");
    settings::on_menu(&mut p, &m.id.clone(), "customfield_10050");
    assert_eq!(p.cfg.sites[0].ac_field("DEF"), Some("customfield_10050"));
}

#[test]
fn a_view_condition_is_built_from_menus_in_words() {
    let mut host = hosted();
    section(&mut host, "views");
    press(&mut host, KeyCode::Down); // My tickets
    press(&mut host, KeyCode::Char('a'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "add a condition");
    press(&mut host, KeyCode::Char('t'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "which type?");
    // Bug, from the tickets seen.
    let at = host.menu.as_ref().unwrap().menu.items.iter().position(|i| i.label == "Bug").unwrap();
    host.menu.as_mut().unwrap().row = at;
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Char('e'));
    let v = &jira(&host).cfg.views[0];
    assert_eq!(v.when, ["assignee:me", "status:open", "-type:Bug"]);
    let got = shown(&host, 160, 40);
    assert!(got.contains("not of type Bug"), "{got}");
    assert!(!got.contains("issuetype"), "{got}");
}

#[test]
fn jql_typed_by_hand_is_kept_only_once_jira_reads_it() {
    let mut host = hosted();
    section(&mut host, "views");
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Char('t'));
    assert!(host.plugins[0].capturing());
    press(&mut host, KeyCode::Char('u'));
    // ctrl-u clears to the start of the line.
    host.update(Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    typed(&mut host, "sprnt = 1");
    let fx = press(&mut host, KeyCode::Enter);
    assert!(host.message.as_deref().is_some_and(|m| m.starts_with("not yet:")));
    assert_eq!(spawned(&fx), 1, "Jira is asked about it");
    let p = jira_mut(&mut host);
    p.on_msg(Box::new(JiraMsg::Jql { text: "sprnt = 1".into(), result: Ok(vec!["The field 'sprnt' does not exist.".into()]) }));
    assert!(shown(&host, 160, 40).contains(":error: Jira says: The field 'sprnt' does not exist."));
    for _ in 0..9 {
        press(&mut host, KeyCode::Backspace);
    }
    typed(&mut host, "project = ABC");
    let p = jira_mut(&mut host);
    p.on_msg(Box::new(JiraMsg::Jql { text: "project = ABC".into(), result: Ok(vec![]) }));
    press(&mut host, KeyCode::Enter);
    assert_eq!(jira(&host).cfg.views[0].jql, "project = ABC");
    assert!(!shown(&host, 160, 40).contains("project = ABC ORDER"), "the nav pane never shows JQL");
}

#[test]
fn look_toggles_and_colours_through_the_menu() {
    let mut host = hosted();
    section(&mut host, "look");
    press(&mut host, KeyCode::Down); // doing
    press(&mut host, KeyCode::Char('o'));
    assert_eq!(jira(&host).cfg.style_for("doing"), "status.info");
    press(&mut host, KeyCode::Char('c'));
    let at = host.menu.as_ref().unwrap().menu.items.iter().position(|i| i.value == "accent.primary").unwrap();
    host.menu.as_mut().unwrap().row = at;
    press(&mut host, KeyCode::Enter);
    assert_eq!(jira(&host).cfg.style_for("doing"), "accent.primary");
    press(&mut host, KeyCode::Char('r'));
    assert_eq!(jira(&host).cfg.style_for("doing"), "status.info bold");
}

#[test]
fn removing_a_site_asks_first() {
    let mut host = hosted();
    section(&mut host, "connection");
    for _ in 0..3 {
        press(&mut host, KeyCode::Down); // past the add row and two offers: acme
    }
    press(&mut host, KeyCode::Char('d'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "stop using acme?");
    press(&mut host, KeyCode::Char('n'));
    assert_eq!(jira(&host).cfg.sites.len(), 2);
    press(&mut host, KeyCode::Char('d'));
    press(&mut host, KeyCode::Char('y'));
    assert_eq!(jira(&host).cfg.sites.len(), 1);
}

#[test]
fn a_check_reports_who_the_token_is() {
    let mut p = sample();
    let fx = settings::on_checked(&mut p, crate::settings_checks::Target::Site("beta".into()), Ok("Alice Example".into()));
    assert!(matches!(&fx[0], Effect::Notify(m) if m == "✓ signed in as Alice Example"));
    assert_eq!(p.data["beta"].me.as_ref().unwrap().name, "Alice Example");
    let fx = settings::on_checked(&mut p, crate::settings_checks::Target::Site("acme".into()), Err("Jira turned the email and API token down (401): check them under settings, sites".into()));
    assert!(matches!(&fx[0], Effect::Notice(m) if m.tone == radar_core::view::chrome::NoticeTone::Error && m.starts_with("✗ Jira turned")));
}

/// Open a settings field's input from its row, empty it and type `w`: the
/// view says the field starts as far into the row as the value did
/// (`Editing::at`; the renderer lays the input over the old value, so that
/// nothing of it stays beside the input: a stray first letter looks typed
/// but cannot be deleted, and ⏎ drops it). `row` picks the line, `value` the text on it
/// where the input starts.
fn input_sits_on_its_value(host: &mut radar_core::Host, name: &str, downs: usize, open: &[KeyCode], row: &str, value: &str) {
    use radar_core::view::Node;
    let key = |host: &mut radar_core::Host, code: KeyCode, m: KeyModifiers| {
        let _ = shown(host, 160, 40);
        host.update(Event::Key(KeyEvent::new(code, m)))
    };
    section(host, name);
    for _ in 0..20 {
        key(host, KeyCode::Up, KeyModifiers::NONE);
    }
    for _ in 0..downs {
        key(host, KeyCode::Down, KeyModifiers::NONE);
    }
    let before = shown(host, 160, 40);
    let _ = radar_core::headless::section_of(host, 160, 40);
    let view = radar_core::headless::section_of(host, 160, 40).expect("the section");
    // The row's text as the view holds it (the text form trims it): where
    // the value starts in it.
    fn names(rows: &[radar_core::view::TreeRow], out: &mut Vec<String>) {
        for r in rows {
            out.push(radar_core::headless::text::text(&r.name));
            names(&r.children, out);
        }
    }
    let (names, tree) = match &view.body {
        Node::List { rows, .. } => (rows.iter().map(|r| radar_core::headless::text::text(&r.text)).collect::<Vec<_>>(), false),
        Node::Tree(t) => {
            let mut out = vec![];
            names(&t.rows, &mut out);
            (out, true)
        }
        _ => panic!("{name}: neither a list nor a tree"),
    };
    let text = names.iter().find(|t| t.as_str() == row).or_else(|| names.iter().find(|t| t.contains(row))).cloned().unwrap_or_else(|| panic!("no row {row:?} in {name}:\n{before}"));
    let start = text.find(row).unwrap();
    let c = match text[start..].find(value) {
        Some(at) => radar_core::design::width::width(&text[..start + at]),
        None => panic!("no {value:?} on {text:?}"),
    };
    for code in open {
        key(host, *code, KeyModifiers::NONE);
    }
    assert!(host.plugins[0].capturing(), "{name}: {open:?} on {row:?} opened no input");
    key(host, KeyCode::Char('e'), KeyModifiers::CONTROL);
    key(host, KeyCode::Char('u'), KeyModifiers::CONTROL);
    // w is text while typing, not the save key.
    let fx = key(host, KeyCode::Char('w'), KeyModifiers::NONE);
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Save(_))), "{name}: w saved instead of being typed");
    let _ = shown(host, 160, 40);
    let view = radar_core::headless::section_of(host, 160, 40).expect("the section");
    let after = shown(host, 160, 40);
    let typed = after.lines().find(|l| l.contains("[w|]")).unwrap_or_else(|| panic!("{name}, {row:?}: no input:\n{after}"));
    let (text, at) = match &view.body {
        Node::List { editing: Some(e), .. } => (e.text.as_str(), usize::from(e.at)),
        Node::Tree(t) if t.editing.is_some() => {
            let e = t.editing.as_ref().unwrap();
            (e.text.as_str(), usize::from(e.at))
        }
        _ => panic!("{name}, {row:?}: the view has no input"),
    };
    // A list row's mark takes one cell before its text; a tree's name starts at 0.
    assert_eq!((text, at), ("w", c + usize::from(!tree)), "{name}, {row:?}: the input does not start where {value:?} was: {typed}");
    key(host, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!host.plugins[0].capturing());
}

#[test]
fn every_settings_input_starts_where_its_value_was_drawn() {
    use KeyCode::{Char, Enter};
    let mut host = hosted();
    // connection: after the two sites found, the add row, a site's name,
    // address, account, token; refresh.
    input_sits_on_its_value(&mut host, "connection", 2, &[Enter], "+ add a site", "add a site");
    input_sits_on_its_value(&mut host, "connection", 3, &[Char('n')], "acme", "acme");
    input_sits_on_its_value(&mut host, "connection", 4, &[Char('e')], "site            ", "https://acme");
    input_sits_on_its_value(&mut host, "connection", 5, &[Char('e')], "account  ", "alice@");
    input_sits_on_its_value(&mut host, "connection", 6, &[Char('e')], "token  ", "env:RADAR");
    input_sits_on_its_value(&mut host, "connection", 9, &[Enter], "projects  ", "ABC");
    input_sits_on_its_value(&mut host, "connection", 17, &[Char('e')], "refresh ", "every 2m");
    // projects: the add row under each site.
    input_sits_on_its_value(&mut host, "projects", 3, &[Char('a')], "+ watch a project", "watch a project");
    input_sits_on_its_value(&mut host, "projects", 8, &[Char('a')], "+ watch a project", "watch a project");
    // views: the add row, a view's name, its JQL, words to add, a type, a label.
    let first = jira(&host).cfg.views[0].name.clone();
    let row = first.clone();
    let shows: String = {
        let mut h = hosted();
        section(&mut h, "views");
        let t = shown(&h, 160, 40);
        let l = t.lines().find(|l| l.contains(&row)).unwrap().to_string();
        l[l.find(&row).unwrap() + row.len()..].trim_start().chars().take(3).collect()
    };
    input_sits_on_its_value(&mut host, "views", 0, &[Enter], "+ new view", "new view");
    input_sits_on_its_value(&mut host, "views", 1, &[Char('r')], &row, &first);
    input_sits_on_its_value(&mut host, "views", 1, &[Char('t')], &row, &shows);
    input_sits_on_its_value(&mut host, "views", 1, &[Char('a'), Char('x')], &row, &shows);
    input_sits_on_its_value(&mut host, "views", 1, &[Char('a'), Char('t'), Char('y')], &row, &shows);
    input_sits_on_its_value(&mut host, "views", 1, &[Char('a'), Char('l'), Char('y')], &row, &shows);
}

// --- keys by context, times and people for the CLI --------------------------------

fn at_ticket(p: &JiraPlugin) {
    p.tree
        .borrow_mut()
        .select(vec!["s:acme".into(), "g:acme:In Progress".into(), "t:acme:ABC-1".into()]);
}

/// Jira's own actions are lower case (capitals are screens); the one capital
/// is `Y`, the shifted partner of `y` that copies the key.
#[test]
fn actions_are_lower_case_and_y_copies_the_key() {
    let mut p = sample();
    at_ticket(&p);
    let mut rows = write::bindings(&p, Pane::Middle);
    rows.extend(screen::keys(&p, Pane::Middle));
    let keys: Vec<&str> = rows.iter().map(|b| b.keys.as_str()).collect();
    for want in ["t", "c", "n", "Y"] {
        assert!(keys.contains(&want), "{want} in {keys:?}");
    }
    assert!(
        keys.iter().all(|k| k.chars().count() != 1 || !k.chars().all(char::is_uppercase) || *k == "Y"),
        "{keys:?}"
    );
    let press = |p: &mut JiraPlugin, c: char| {
        p.intent("tickets", Pane::Middle, &radar_core::view::Intent::Key { chord: radar_core::view::Chord::char(c) })
    };
    let out = press(&mut p, 'Y');
    assert!(matches!(&out, Outcome::Handled(fx) if matches!(&fx[0], Effect::Copy(k) if k == "ABC-1")), "{out:?}");
    // `c` comments (the composer takes the keys), it never copies.
    let out = press(&mut p, 'c');
    assert!(!matches!(&out, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::Copy(_)))), "{out:?}");
}

/// An open ticket page is read again when the list says its ticket changed
/// (its `updated` moved), never only because time passed.
#[test]
fn an_open_page_is_read_again_only_when_its_ticket_changed() {
    let mut p = sample();
    page::open(&mut p, "acme", "ABC-12");
    crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(detail()));
    let id = detail_key("acme", "ABC-12");
    p.tick(p.now);
    assert!(!p.details[&id].loading, "fresh: left alone");
    p.tick(p.now + p.cfg.refresh_secs as i64 * 10);
    assert!(!p.details[&id].loading, "unchanged: no detail fetch however long");
    p.data.get_mut("acme").unwrap().tickets.get_mut("ABC-12").unwrap().updated = "2026-10-07T09:00:00.000+0000".into();
    p.tick(p.now + 1);
    assert!(p.details[&id].loading, "changed: asked again");
}

/// The CLI's JSON: RFC 3339 times and people as `{name, handle}`.
#[test]
fn json_times_are_rfc_3339_and_people_have_a_name_and_a_handle() {
    assert_eq!(screen::rfc3339("2026-10-06T09:00:00.000+0000"), "2026-10-06T09:00:00Z");
    assert_eq!(screen::rfc3339("not a time"), "not a time");
    assert_eq!(screen::person("Alice Example", "acc-1"), serde_json::json!({"name": "Alice Example", "handle": "acc-1"}));
    assert_eq!(screen::person("Bob", ""), serde_json::json!({"name": "Bob", "handle": null}));
    assert!(screen::person("", "").is_null());
}

/// The old section ids still find their section.
#[test]
fn old_section_ids_still_find_their_section() {
    assert_eq!((canon("sites"), canon("jira-views"), canon("jira-look")), ("connection", "views", "look"));
    assert_eq!(canon("projects"), "projects");
    assert_eq!(SECTIONS.iter().map(|s| (s.id, s.name)).filter(|(i, n)| i != n).count(), 0, "id = name");
}

#[test]
fn an_action_is_invoked_by_its_stable_id_through_headless() {
    use radar_core::view::{Intent, PaneId};
    let mut headless = radar_core::headless::Headless::new(hosted(), 160, 40);
    headless.focus(PaneId::Middle);
    let ids: Vec<String> = headless.screen().unwrap().actions.iter().map(|a| a.id.clone()).collect();
    assert!(ids.contains(&"jira.tickets.filter".to_string()), "{ids:?}");
    assert!(ids.iter().all(|id| id.is_empty() || id.starts_with("jira.tickets.")), "{ids:?}");
    assert!(jira(headless.host()).filter_input.is_none());
    headless.send(Intent::Invoke { action: "jira.tickets.filter".into() });
    assert!(jira(headless.host()).filter_input.is_some(), "the id ran the filter row");
}
