//! Jira's step in setup (`radar setup jira`). Its card is the `connection`
//! section, the rows Settings edits: a Jira set found in the environment,
//! each site with its address, account, token, who it signs in as, the
//! check and the projects watched there. Its sheet is the selected row's
//! detail, then the help that row needs: the three things Jira asks for
//! (site, email, API token), or what a project to watch can be.

use radar_core::plugin::{SetupFinds, SetupGroup, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Seg, Text};

use crate::JiraPlugin;
use crate::config;

/// radar's guide to connecting Jira.
const GUIDE: &str = "https://github.com/temet-ai/radar/blob/main/docs/user-guide/connect.md#jira";
/// Atlassian's API token page.
pub(crate) const TOKEN_PAGE: &str = "https://id.atlassian.com/manage-profile/security/api-tokens";
/// Where the account's email shows.
const PROFILE: &str = "https://id.atlassian.com/manage-profile/profile-and-visibility";
const DOCS: &str = "https://support.atlassian.com/atlassian-account/docs/manage-api-tokens-for-your-atlassian-account/";

pub(crate) fn step(p: &JiraPlugin) -> SetupStep {
    let step = SetupStep::new("jira", "Jira", "connection")
        .heading("Connect Jira.", "Issues from the projects you follow, in saved views.")
        .icon(Icon::Issue)
        .about("issues, saved views")
        .group(SetupGroup::Service);
    match summary(p) {
        Some(s) => step.summary(s),
        None => step,
    }
}

/// `Sam Vimes · 2 projects`: who the first site signs in as, and how many
/// projects are watched (and on how many sites, when more than one).
pub(crate) fn summary(p: &JiraPlugin) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(me) = p.cfg.sites.first().and_then(|s| p.data.get(&s.name)).and_then(|d| d.me.as_ref())
        && !me.name.is_empty()
    {
        parts.push(me.name.clone());
    }
    if p.cfg.sites.len() > 1 {
        parts.push(format!("{} sites", p.cfg.sites.len()));
    }
    match p.cfg.sites.iter().map(|s| s.projects.len()).sum::<usize>() {
        0 => {}
        1 => parts.push("1 project".into()),
        n => parts.push(format!("{n} projects")),
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Where setup may look, once asked: every `JIRA_…` variable (the site, the
/// account and the token are read from such a set).
pub(crate) fn finds() -> SetupFinds {
    SetupFinds { env: vec!["JIRA_"], files: vec![] }
}

fn line(words: &str) -> Block {
    Block::line(Text::of(words, Ink::Secondary))
}

fn command(cmd: &str) -> Block {
    Block::line(Text::new().push(Seg::new("$ ", Ink::Muted)).push(Seg::new(cmd, Ink::Primary)))
}

fn link(words: &str, url: &str, hints: &[(&str, &str)]) -> Block {
    let mut text = Text::new().push(Seg::new(words, Ink::Accent).linked(url));
    for (index, (key, action)) in hints.iter().enumerate() {
        text.add(Seg::new(if index == 0 { "   " } else { " · " }, Ink::Muted));
        text.extend(radar_core::mnemonic::text(key, action, "text.muted"));
    }
    Block::line(text)
}

/// What Jira needs, and where each comes from.
pub(crate) fn access_help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Site), "site", None, vec![
            line("The address you use for Jira, like ankh-morpork.atlassian.net."),
            line("Paste any Jira link; radar keeps the site part."),
        ]),
        Block::section(Some(Icon::Person), "email", None, vec![
            line("The email you sign in to Atlassian with, on your profile:"),
            link("id.atlassian.com / profile", PROFILE, &[]),
        ]),
        Block::section(Some(Icon::Connection), "API token", Some("about a minute".into()), vec![
            line("1 Open Atlassian's API token page:"),
            link("id.atlassian.com / api tokens", TOKEN_PAGE, &[("o", "open"), ("y", "copy")]),
            line("2 Create API token, label it radar, pick an expiry, then Create."),
            line("3 Copy it now: Atlassian shows it once. Paste it into token, or:"),
            command(&format!("export {}=…", config::DEFAULT_TOKEN)),
            line(&format!("  then the token is env:{}", config::DEFAULT_TOKEN)),
        ]),
        Block::section(Some(Icon::Warning), "if it is refused", None, vec![
            line("401: the email and token don't match, or the token expired."),
            line("Company login (SSO)? An API token still works; ask your admin if API tokens are switched off."),
        ]),
        more(),
    ]
}

/// What a project to watch can be.
pub(crate) fn projects_help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Issue), "projects", None, vec![
            line("Type a project key, like WATCH, or paste a link to the project."),
            Block::line(radar_core::mnemonic::text("p", "pick from the projects you looked at lately.", "text.secondary")),
            line("Colours, checks and acceptance criteria: Settings / Jira / projects."),
            line("Saved queries: Settings / Jira / views."),
        ]),
        more(),
    ]
}

fn more() -> Block {
    Block::section(Some(Icon::Link), "more", None, vec![
        link("Atlassian: manage API tokens", DOCS, &[]),
        link("radar guide: connect Jira", GUIDE, &[]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_is_the_connection_and_counts_the_projects() {
        let mut p = crate::demo::sample();
        let step = step(&p);
        assert_eq!((step.id, step.section, step.group), ("jira", "connection", SetupGroup::Service));
        let n: usize = p.cfg.sites.iter().map(|s| s.projects.len()).sum();
        assert!(summary(&p).is_some_and(|s| s.ends_with(&format!("{n} projects")) || n == 1));
        p.cfg.sites.clear();
        assert_eq!(summary(&p), None);
        assert_eq!(finds().env, vec!["JIRA_"]);
    }

    /// As a setup step the connection card leaves out refresh and on start; ↓
    /// walks only the rows that are drawn.
    #[test]
    fn the_setup_card_leaves_out_refresh_and_on_start_and_the_cursor_skips_nothing_drawn() {
        use crate::settings::{SiteRow, key, site_rows};
        use radar_core::plugin::Plugin;
        use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use radar_core::view::{Node, ViewCx};
        let cx = |setup| ViewCx {
            now: 0,
            focus: radar_core::view::PaneId::Middle,
            width: 100,
            height: 40,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup,
        };
        let drawn = |p: &JiraPlugin, setup| -> Vec<String> {
            let view = p.section_view("connection", &cx(setup)).expect("the connection section");
            let Node::Tree(tree) = view.body else { panic!("a tree") };
            let words = |r: &radar_core::view::TreeRow| r.name.0.iter().map(|s| s.text.as_str()).collect::<String>();
            tree.rows.iter().flat_map(|r| std::iter::once(words(r)).chain(r.children.iter().map(words))).collect()
        };
        let has = |rows: &[String], label: &str| rows.iter().any(|r| r.contains(label));

        let mut p = crate::demo::sample();
        let settings = drawn(&p, false);
        for label in ["refresh", "on start", "token"] {
            assert!(has(&settings, label), "Settings keeps {label}: {settings:#?}");
        }
        let in_settings = site_rows(&p).len();
        assert_eq!(settings.len(), in_settings);

        let card = drawn(&p, true);
        for label in ["refresh", "on start"] {
            assert!(!has(&card, label), "setup leaves out {label}: {card:#?}");
        }
        for label in ["token", "add a site"] {
            assert!(has(&card, label), "setup keeps {label}: {card:#?}");
        }
        let rows = site_rows(&p);
        assert_eq!(card.len(), rows.len(), "the rows keys walk are the rows drawn");
        assert!(!rows.contains(&SiteRow::Refresh) && !rows.contains(&SiteRow::OnStart));

        // ↓ through the whole card never lands on a row that is not drawn.
        p.section_rows[0] = 0;
        for _ in 0..rows.len() + 2 {
            key(&mut p, "connection", KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            let at = p.section_rows[0];
            assert!(at < card.len(), "row {at} is not drawn");
            assert!(!matches!(rows[at], SiteRow::Refresh | SiteRow::OnStart));
        }
        assert_eq!(p.section_rows[0], rows.len() - 1, "it ends on the last drawn row");

        // Back in Settings the rows come back.
        let again = drawn(&p, false);
        assert!(has(&again, "refresh") && has(&again, "on start"));
        assert_eq!(site_rows(&p).len(), in_settings);
    }

    /// Each site's projects row watches another with the projects
    /// section's own field; the sheet says what Jira needs.
    #[test]
    fn a_sites_projects_row_watches_with_the_projects_own_field() {
        use crate::settings::{Field, SiteRow, connection_view, key, site_rows};
        use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut p = crate::demo::sample();
        let press = |p: &mut JiraPlugin, code| key(p, "connection", KeyEvent::new(code, KeyModifiers::NONE));
        p.section_rows[0] = site_rows(&p).iter().position(|r| *r == SiteRow::Field(0, Field::Projects)).unwrap();
        press(&mut p, KeyCode::Enter);
        assert!(matches!(p.edit, Some((crate::Target::ProjectAdd(0), _))));
        let before = p.cfg.sites[0].projects.len();
        for c in "WATCH".chars() {
            press(&mut p, KeyCode::Char(c));
        }
        press(&mut p, KeyCode::Enter);
        assert_eq!(p.cfg.sites[0].projects.len(), before + 1);
        assert!(p.cfg.sites[0].watches("WATCH"));
        p.section_rows[0] = site_rows(&p).iter().position(|r| *r == SiteRow::Field(0, Field::Token)).unwrap();
        let sheet = format!("{:?}", connection_view(&p, 80, 0).detail_blocks);
        assert!(sheet.contains("API TOKEN") && sheet.contains(TOKEN_PAGE));
    }
}
