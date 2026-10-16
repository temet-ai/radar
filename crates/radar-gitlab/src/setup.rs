//! GitLab's step in setup (`radar setup gitlab`). Its card is the
//! `connection` section, the same rows Settings edits: a token found in
//! the environment, the server and the token, who it signs in as, the
//! check, and the groups followed. Its sheet is the selected row's detail,
//! then the help that row needs: where to get a token (the new-token page,
//! `glab`, an own server, a refusal) or what a group to follow can be.

use radar_core::plugin::{SetupFinds, SetupGroup, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Seg, Text};

use crate::GitlabPlugin;
use crate::config::TokenAccess;

/// radar's guide to connecting GitLab.
pub(crate) const GUIDE: &str = "https://github.com/temet-ai/radar/blob/main/docs/user-guide/connect.md#gitlab";
/// GitLab's own page on access tokens.
const DOCS: &str = "https://docs.gitlab.com/user/profile/personal_access_tokens/";

/// The step: one stop on the rail, one row in the chooser.
pub(crate) fn step(p: &GitlabPlugin) -> SetupStep {
    let step = SetupStep::new("gitlab", "GitLab", "connection")
        .heading("Connect GitLab.", "Merge requests, pipelines and reviews from the groups you follow.")
        .icon(Icon::Group)
        .about("merge requests, pipelines")
        .group(SetupGroup::Service);
    match summary(p) {
        Some(s) => step.summary(s),
        None => step,
    }
}

/// `@vimes · 2 groups`: who the token signs in as, and how many are
/// followed, then `read and write` when the token is made to write;
/// `None` while neither who nor how many is known.
pub(crate) fn summary(p: &GitlabPlugin) -> Option<String> {
    let mut parts = Vec::new();
    if !p.me.is_empty() {
        parts.push(format!("@{}", p.me));
    }
    match p.cfg.sources.len() {
        0 => {}
        1 => parts.push("1 group".into()),
        n => parts.push(format!("{n} groups")),
    }
    if !parts.is_empty() && p.cfg.access == TokenAccess::Write {
        parts.push(TokenAccess::Write.label().into());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Where setup may look, once asked: the variable radar reads by default
/// and glab's sign-in (its file on Linux, and on the Mac).
pub(crate) fn finds() -> SetupFinds {
    SetupFinds {
        env: vec!["GITLAB_TOKEN"],
        files: vec![
            ("glab", ".config/glab-cli/config.yml"),
            ("glab", "Library/Application Support/glab-cli/config.yml"),
        ],
    }
}

/// The scope a new token asks for, from the access chosen: `read_api` to
/// read, `api` to also comment, approve and merge.
pub(crate) fn scope(p: &GitlabPlugin) -> &'static str {
    p.cfg.access.scope()
}

/// What the check adds when the token's scopes, as GitLab told them, lack
/// what the access chosen needs: ` · this token has read_api only; read
/// and write needs api`. Nothing when they are enough or unknown.
pub(crate) fn scope_note(access: TokenAccess, scopes: Option<&[String]>) -> Option<String> {
    let scopes = scopes?;
    let need = access.scope();
    // `api` covers everything `read_api` does.
    if scopes.iter().any(|s| s == need || s == "api") {
        return None;
    }
    let has = if scopes.is_empty() { "no scopes".to_string() } else { format!("{} only", scopes.join(", ")) };
    Some(format!(" · this token has {has}; {} needs {need}", access.label()))
}

/// The server's new-token page, with radar's name and the scope filled in.
pub(crate) fn token_page(p: &GitlabPlugin) -> String {
    format!(
        "{}/-/user_settings/personal_access_tokens?name=radar&scopes={}",
        p.cfg.url.trim_end_matches('/'),
        scope(p)
    )
}

/// `gitlab.com / new token`: the page's host, as the link's words.
fn token_page_words(p: &GitlabPlugin) -> String {
    let host = p
        .cfg
        .url
        .split("://")
        .nth(1)
        .unwrap_or(&p.cfg.url)
        .trim_end_matches('/');
    format!("{host} / new token")
}

fn line(words: &str) -> Block {
    Block::line(Text::of(words, Ink::Secondary))
}

/// A `$ command` line.
fn command(cmd: &str) -> Block {
    Block::line(Text::new().push(Seg::new("$ ", Ink::Muted)).push(Seg::new(cmd, Ink::Primary)))
}

/// A link: its words, opened when clicked; then the keys that open and
/// copy it from the section, when it has them.
fn link(words: &str, url: &str, keys: Option<&str>) -> Block {
    let mut text = Text::new().push(Seg::new(words, Ink::Accent).linked(url));
    if let Some(keys) = keys {
        text.add(Seg::new(format!("   {keys}"), Ink::Muted));
    }
    Block::line(text)
}

/// How to get a token: the new-token page, glab, an own server, a refusal,
/// and where to read more.
pub(crate) fn token_help(p: &GitlabPlugin) -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Connection), "get a GitLab token", None, vec![
            line("1 Open GitLab's new-token page:"),
            link(&token_page_words(p), &token_page(p), Some("o opens · y copies")),
            line(&format!("  name radar · scope from access: {}", scope(p))),
            line("2 Pick an expiry date, then Create personal access token."),
            line("3 Copy the glpat-… value into token, or keep it out of radar's config:"),
            command("export GITLAB_TOKEN=glpat-…"),
            line("  then the token is env:GITLAB_TOKEN"),
        ]),
        Block::section(Some(Icon::Terminal), "already use glab?", None, vec![
            line("Type glab as the token. Not signed in?"),
            command("glab auth login"),
        ]),
        Block::section(Some(Icon::Site), "your own GitLab", None, vec![line(
            "Set server first; the link above then points at your server's page.",
        )]),
        Block::section(Some(Icon::Warning), "if it is refused", None, vec![
            line(&format!("Expired, or without {}: make a new one with the link.", scope(p))),
            line("radar shows GitLab's own reason."),
        ]),
        more(),
    ]
}

/// What a group to follow can be.
pub(crate) fn follow_help(p: &GitlabPlugin) -> Vec<Block> {
    let server = p.cfg.url.trim_end_matches('/');
    vec![
        Block::section(Some(Icon::Group), "follow", None, vec![
            line("Paste or type any of:"),
            line(&format!("  {server}/ankh-morpork/watch")),
            line("  ankh-morpork/watch"),
            line("  a repository's link or path"),
            line("Labels, colours and hidden repositories: Settings / GitLab / tenancies."),
        ]),
        more(),
    ]
}

fn more() -> Block {
    Block::section(Some(Icon::Link), "more", None, vec![
        link("GitLab docs: access tokens", DOCS, None),
        link("radar guide: connect GitLab", GUIDE, None),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample;

    #[test]
    fn the_step_is_the_connection_section_and_says_who_and_how_many() {
        let mut p = sample();
        p.me = "vimes".into();
        let step = step(&p);
        assert_eq!((step.id, step.section, step.group), ("gitlab", "connection", SetupGroup::Service));
        let groups = p.cfg.sources.len();
        assert_eq!(step.summary, Some(format!("@vimes · {groups} groups")));
        p.cfg.access = TokenAccess::Write;
        assert_eq!(summary(&p), Some(format!("@vimes · {groups} groups · read and write")));
        p.me.clear();
        p.cfg.sources.clear();
        assert_eq!(summary(&p), None, "nothing to say before a sign-in or a group");
    }

    /// As a setup step the connection card leaves out base group, refresh and
    /// merged, closed; ↓ walks only the rows that are drawn.
    #[test]
    fn the_setup_card_leaves_out_what_only_settings_needs_and_the_cursor_skips_nothing_drawn() {
        use crate::settings::{ConnRow, Field, conn_rows, key};
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
        let drawn = |p: &GitlabPlugin, setup| -> Vec<String> {
            let view = p.section_view("connection", &cx(setup)).expect("the connection section");
            let Node::List { rows, .. } = view.body else { panic!("a list") };
            rows.iter().map(|r| r.text.0.iter().map(|s| s.text.as_str()).collect::<String>()).collect()
        };
        let has = |rows: &[String], label: &str| rows.iter().any(|r| r.trim_start().starts_with(label));

        let mut p = sample();
        let settings = drawn(&p, false);
        for label in ["base group", "refresh", "merged, closed", "server", "token"] {
            assert!(has(&settings, label), "Settings keeps {label}: {settings:#?}");
        }
        let in_settings = conn_rows(&p).len();
        assert_eq!(settings.len(), in_settings);

        let card = drawn(&p, true);
        for label in ["base group", "refresh", "merged, closed"] {
            assert!(!has(&card, label), "setup leaves out {label}: {card:#?}");
        }
        for label in ["server", "token", "ACCESS", "FOLLOW"] {
            assert!(has(&card, label), "setup keeps {label}: {card:#?}");
        }
        let rows = conn_rows(&p);
        assert_eq!(card.len(), rows.len(), "the rows keys walk are the rows drawn");
        assert!(!rows.iter().any(|r| matches!(r, ConnRow::Field(Field::Base | Field::Refresh | Field::History))));

        // ↓ through the whole card never lands on a hidden row or a heading.
        p.section_rows[0] = 0;
        for _ in 0..rows.len() + 2 {
            key(&mut p, "connection", KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            let row = rows[p.section_rows[0]];
            assert!(!matches!(row, ConnRow::Gap | ConnRow::AccessHeading | ConnRow::FollowHeading), "{row:?}");
            assert!(!matches!(row, ConnRow::Field(Field::Base | Field::Refresh | Field::History)));
        }

        // Back in Settings the rows come back.
        let again = drawn(&p, false);
        assert!(has(&again, "refresh") && has(&again, "base group"));
        assert_eq!(conn_rows(&p).len(), in_settings);
    }

    #[test]
    fn it_looks_for_names_only_and_the_page_fills_in_the_scope() {
        let f = finds();
        assert_eq!(f.env, vec!["GITLAB_TOKEN"]);
        assert!(f.files.iter().all(|(program, path)| *program == "glab" && !path.starts_with('/')));
        let mut p = sample();
        p.cfg.url = "https://git.ankh-morpork.example/".into();
        assert_eq!(
            token_page(&p),
            "https://git.ankh-morpork.example/-/user_settings/personal_access_tokens?name=radar&scopes=read_api"
        );
        assert_eq!(token_page_words(&p), "git.ankh-morpork.example / new token");
        p.cfg.access = TokenAccess::Write;
        assert!(token_page(&p).ends_with("scopes=api"));
        // The mode no longer picks the scope: the access does.
        radar_core::writes::set_mode(radar_core::writes::Mode::ReadOnly, None);
        assert!(token_page(&p).ends_with("scopes=api"));
        radar_core::writes::set_mode(radar_core::writes::Mode::Edit, None);
        p.cfg.access = TokenAccess::Read;
        assert!(token_page(&p).ends_with("scopes=read_api"));
    }

    #[test]
    fn the_check_says_when_the_tokens_scopes_lack_what_the_access_needs() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let read_api = s(&["read_api", "read_user"]);
        assert_eq!(
            scope_note(TokenAccess::Write, Some(&read_api)),
            Some(" · this token has read_api, read_user only; read and write needs api".into())
        );
        assert_eq!(scope_note(TokenAccess::Write, Some(&s(&["api"]))), None);
        assert_eq!(scope_note(TokenAccess::Read, Some(&read_api)), None);
        assert_eq!(scope_note(TokenAccess::Read, Some(&s(&["api"]))), None, "api reads too");
        assert_eq!(scope_note(TokenAccess::Write, Some(&[])), Some(" · this token has no scopes; read and write needs api".into()));
        assert_eq!(scope_note(TokenAccess::Write, None), None, "unknown is not invented");
    }
}
