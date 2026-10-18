//! GitHub's step in setup (`radar setup github`). Its card is the
//! `connection` section, the rows Settings edits: how radar signs in
//! (public only, gh's sign-in, or a token), the server and the token, the
//! check, and the repositories followed. Its sheet is the selected row's
//! detail, then the help that row needs: gh, or a fine-grained token.

use radar_core::plugin::{SetupFinds, SetupGroup, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Seg, Text};

use crate::GithubPlugin;
use crate::config::TokenAccess;

/// radar's guide to connecting GitHub.
const GUIDE: &str = "https://github.com/temet-ai/radar/blob/main/docs/user-guide/connect.md#github";
/// GitHub's own page on tokens.
const DOCS: &str = "https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens";
/// Where gh is installed from.
const GH_SITE: &str = "https://cli.github.com/";

pub(crate) fn step(p: &GithubPlugin) -> SetupStep {
    let step = SetupStep::new("github", "GitHub", "connection")
        .heading("Connect GitHub.", "Pull requests, their checks, and Actions runs.")
        .icon(Icon::Repo)
        .about("pull requests, checks")
        .group(SetupGroup::Service);
    match summary(p) {
        Some(s) => step.summary(s),
        None => step,
    }
}

/// `@vimes · 3 repos`: who the check signed in as (`public` with no
/// token), and how many repositories are followed.
pub(crate) fn summary(p: &GithubPlugin) -> Option<String> {
    let repos: usize = p.cfg.groups.iter().map(|g| g.repositories.len()).sum();
    let who = match (&p.login, p.cfg.token.is_empty()) {
        (Some(login), _) => Some(format!("@{login}")),
        (None, true) if repos > 0 => Some("public".to_string()),
        _ => None,
    };
    let mut parts: Vec<String> = who.into_iter().collect();
    match repos {
        0 => {}
        1 => parts.push("1 repo".into()),
        n => parts.push(format!("{n} repos")),
    }
    if !parts.is_empty() && p.cfg.access == TokenAccess::Write {
        parts.push(TokenAccess::Write.label().into());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What the check adds when a token's OAuth scopes, as GitHub sent them
/// with the check (a classic token or gh's), lack `repo`, which reading
/// and writing private repositories needs. Nothing for read, or when
/// GitHub did not say (a fine-grained token's permissions are not told).
pub(crate) fn scope_note(access: TokenAccess, scopes: Option<&[String]>) -> Option<String> {
    let scopes = scopes?;
    if access == TokenAccess::Read || scopes.iter().any(|s| s == "repo") {
        return None;
    }
    let has = if scopes.is_empty() { "no scopes".to_string() } else { format!("scopes {} only", scopes.join(", ")) };
    Some(format!(" · this token has {has}; {} needs repo", access.label()))
}

/// Where setup may look, once asked: the two variables gh and GitHub's
/// own tools read, and gh's sign-in.
pub(crate) fn finds() -> SetupFinds {
    SetupFinds { env: vec!["GITHUB_TOKEN", "GH_TOKEN"], files: vec![("gh", ".config/gh/hosts.yml")] }
}

/// The fine-grained token page, with radar's name and the permissions the
/// access chosen needs filled in.
pub(crate) fn token_page(p: &GithubPlugin) -> String {
    let host = crate::config::Endpoint::parse(&p.cfg.api).map(|api| api.gh_host()).unwrap_or_else(|_| "github.com".into());
    let permissions: String = p.cfg.access.permissions().iter().map(|(what, level)| format!("&{what}={level}")).collect();
    format!("https://{host}/settings/personal-access-tokens/new?name=radar&description=radar{permissions}")
}

/// The permissions line of the token help, as the page fills them in.
fn permissions_words(access: TokenAccess) -> &'static str {
    match access {
        TokenAccess::Read => "contents, pull requests, actions, statuses: read",
        TokenAccess::Write => "contents, pull requests, actions: read and write; statuses: read",
    }
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

/// How to sign in: gh, else a fine-grained token, and an organisation's.
pub(crate) fn access_help(p: &GithubPlugin) -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Terminal), "easiest: the gh program", None, vec![
            line("Choose gh's sign-in. Not signed in yet?"),
            command("gh auth login"),
            link("cli.github.com: install gh", GH_SITE, &[]),
        ]),
        Block::section(Some(Icon::Connection), "or a token", None, vec![
            line("Public repositories need none. For private ones:"),
            line("1 Open the fine-grained token page:"),
            link("github.com / new token", &token_page(p), &[("o", "open"), ("y", "copy")]),
            line("2 Repository access: the repositories to follow."),
            line(&format!("3 Permissions come filled in from access: {}.", permissions_words(p.cfg.access))),
            line("4 Generate token, copy the github_pat_… value into token, or:"),
            command("export GITHUB_TOKEN=…"),
            line("  then the token is env:GITHUB_TOKEN"),
        ]),
        Block::section(Some(Icon::Team), "an organisation's repositories", None, vec![
            line("Choose the organisation as the token's resource owner."),
            line("Some need an owner to approve the token."),
        ]),
        more(),
    ]
}

/// What can be followed.
pub(crate) fn follow_help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Repo), "follow", None, vec![
            line("Type or paste any of:"),
            line("  github.com/ankh-morpork: an owner's repositories"),
            line("  ankh-morpork/watch: one repository"),
            line("  The Watch: a group of your own"),
            line("With a token, a group of an owner opens a picker of what it can read."),
            line("At most 200 repositories. Read on start and on r; never polled."),
        ]),
        more(),
    ]
}

fn more() -> Block {
    Block::section(Some(Icon::Link), "more", None, vec![
        link("GitHub docs: access tokens", DOCS, &[]),
        link("radar guide: connect GitHub", GUIDE, &[]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As a setup step the connection card leaves out server and refresh; ↓
    /// walks only the rows that are drawn.
    #[test]
    fn the_setup_card_leaves_out_server_and_refresh_and_the_cursor_skips_nothing_drawn() {
        use crate::settings::{ConnRow, conn_rows, key};
        use radar_core::plugin::Plugin;
        use radar_core::view::{Chord, Key, Node, ViewCx};
        let cx = |setup| ViewCx {
            now: 0,
            focus: radar_core::view::PaneId::Middle,
            width: 100,
            height: 40,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup,
        };
        let drawn = |p: &GithubPlugin, setup| -> Vec<String> {
            let view = p.section_view("connection", &cx(setup)).expect("the connection section");
            let Node::List { rows, .. } = view.body else { panic!("a list") };
            rows.iter().map(|r| r.text.0.iter().map(|s| s.text.as_str()).collect::<String>()).collect()
        };
        let has = |rows: &[String], label: &str| rows.iter().any(|r| r.trim_start().starts_with(label));

        let mut p = crate::demo::sample();
        let settings = drawn(&p, false);
        for label in ["server", "refresh", "token"] {
            assert!(has(&settings, label), "Settings keeps {label}: {settings:#?}");
        }
        let in_settings = conn_rows(&p).len();
        assert_eq!(settings.len(), in_settings);

        let card = drawn(&p, true);
        for label in ["server", "refresh"] {
            assert!(!has(&card, label), "setup leaves out {label}: {card:#?}");
        }
        for label in ["token", "SIGN IN", "ACCESS", "REPOSITORIES"] {
            assert!(has(&card, label), "setup keeps {label}: {card:#?}");
        }
        let rows = conn_rows(&p);
        assert_eq!(card.len(), rows.len(), "the rows keys walk are the rows drawn");
        assert!(!rows.contains(&ConnRow::Server) && !rows.contains(&ConnRow::Refresh));

        // ↓ through the whole card never lands on a hidden row or a heading.
        p.connection_row = 0;
        for _ in 0..rows.len() + 2 {
            key(&mut p, "connection", Chord::new(Key::Down));
            let row = rows[crate::settings::conn_at(&p)];
            assert!(row.selectable(), "{row:?}");
            assert!(!matches!(row, ConnRow::Server | ConnRow::Refresh));
        }

        // Back in Settings the rows come back.
        let again = drawn(&p, false);
        assert!(has(&again, "server") && has(&again, "refresh"));
        assert_eq!(conn_rows(&p).len(), in_settings);
    }

    #[test]
    fn the_step_is_the_connection_and_says_who_and_how_many() {
        let mut p = crate::demo::sample();
        let repos: usize = p.cfg.groups.iter().map(|g| g.repositories.len()).sum();
        let step = step(&p);
        assert_eq!((step.id, step.section, step.group), ("github", "connection", SetupGroup::Service));
        p.login = Some("vimes".into());
        assert_eq!(summary(&p), Some(format!("@vimes · {repos} repos")));
        p.login = None;
        p.cfg.token.clear();
        assert_eq!(summary(&p), Some(format!("public · {repos} repos")));
        p.cfg.groups.clear();
        assert_eq!(summary(&p), None);
        assert_eq!(finds().env, vec!["GITHUB_TOKEN", "GH_TOKEN"]);
    }

    /// The card's sign-in choices set the token the same way the token row
    /// does: gh's sign-in is the word gh, checked at once; public is none.
    #[test]
    fn the_sign_in_choices_set_the_token_and_check_it() {
        use crate::settings::{Access, ConnRow, conn_rows, key};
        use radar_core::plugin::{Effect, Outcome};
        use radar_core::view::{Chord, Key};
        let mut p = crate::demo::sample();
        let at = |p: &GithubPlugin, row| conn_rows(p).iter().position(|r| *r == row).unwrap();
        p.connection_row = at(&p, ConnRow::Access(Access::Gh));
        let out = key(&mut p, "connection", Chord::new(Key::Enter));
        assert_eq!(p.cfg.token, crate::auth::GH);
        assert!(matches!(&out, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))), "checked");
        p.connection_row = at(&p, ConnRow::Access(Access::Public));
        key(&mut p, "connection", Chord::new(Key::Enter));
        assert!(p.cfg.token.is_empty());
        p.connection_row = at(&p, ConnRow::Access(Access::Token));
        key(&mut p, "connection", Chord::new(Key::Enter));
        assert!(p.editor.is_some(), "a token is typed into the token row");
        assert_eq!(crate::settings::conn_row(&p), ConnRow::Token);
        p.editor = None;
        p.connection_row = at(&p, ConnRow::FollowAdd);
        key(&mut p, "connection", Chord::new(Key::Up));
        assert_eq!(crate::settings::conn_row(&p), ConnRow::Grant(TokenAccess::Write), "over the gap and the heading");
        p.connection_row = at(&p, ConnRow::Grant(TokenAccess::Read));
        key(&mut p, "connection", Chord::new(Key::Up));
        assert_eq!(crate::settings::conn_row(&p), ConnRow::Refresh);
        let out = key(&mut p, "connection", Chord::new(Key::Char('o')));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.as_slice(), [Effect::Open(u)] if u.contains("personal-access-tokens/new"))));
    }

    /// The access choice fills in the new-token page's permissions, is
    /// kept in the config, and asks a check already made again.
    #[test]
    fn the_access_choice_sets_the_permissions_the_token_link_fills_in() {
        use crate::settings::{ConnRow, conn_rows, key};
        use radar_core::plugin::{Effect, Outcome, Plugin};
        use radar_core::view::{Chord, Key};
        let mut p = crate::demo::sample();
        let at = |p: &GithubPlugin, row| conn_rows(p).iter().position(|r| *r == row).unwrap();
        assert!(token_page(&p).ends_with("&contents=read&pull_requests=read&actions=read&statuses=read"), "{}", token_page(&p));
        p.connection_row = at(&p, ConnRow::Grant(TokenAccess::Write));
        let ids: Vec<String> = crate::settings::keys(&p, "connection").into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&"github.connection.choose".to_string()), "{ids:?}");
        let out = key(&mut p, "connection", Chord::new(Key::Enter));
        assert_eq!(p.cfg.access, TokenAccess::Write);
        assert!(matches!(&out, Outcome::Handled(fx) if !fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))), "nothing checked yet: nothing asked");
        assert!(p.config().is_some_and(|c| c["access"].as_str() == Some("write")), "saved");
        assert!(token_page(&p).ends_with("&contents=write&pull_requests=write&actions=write&statuses=read"), "{}", token_page(&p));
        let help = format!("{:?}", access_help(&p));
        assert!(help.contains("filled in from access: contents, pull requests, actions: read and write; statuses: read"), "{help}");
        let out = key(&mut p, "connection", Chord::new(Key::Char('o')));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.as_slice(), [Effect::Open(u)] if u.contains("pull_requests=write"))));
        p.login = Some("vimes".into());
        assert!(summary(&p).is_some_and(|s| s.ends_with(" · read and write")), "{:?}", summary(&p));

        p.connection_check = Some(radar_core::ui::status::Check::NotYet);
        p.connection_row = at(&p, ConnRow::Grant(TokenAccess::Read));
        let out = key(&mut p, "connection", Chord::new(Key::Enter));
        assert_eq!(p.cfg.access, TokenAccess::Read);
        assert!(matches!(&out, Outcome::Handled(fx) if fx.iter().any(|e| matches!(e, Effect::SpawnAction { .. }))), "checked again");
        assert!(p.config().is_some_and(|c| c.get("access").is_none()), "read is the default, left out");
    }

    #[test]
    fn the_check_says_when_a_classic_tokens_scopes_lack_repo() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            scope_note(TokenAccess::Write, Some(&s(&["read:org", "public_repo"]))),
            Some(" · this token has scopes read:org, public_repo only; read and write needs repo".into())
        );
        assert_eq!(scope_note(TokenAccess::Write, Some(&s(&["repo", "workflow"]))), None);
        assert_eq!(scope_note(TokenAccess::Write, Some(&[])), Some(" · this token has no scopes; read and write needs repo".into()));
        assert_eq!(scope_note(TokenAccess::Read, Some(&[])), None);
        assert_eq!(scope_note(TokenAccess::Write, None), None, "a fine-grained token's permissions are not told");
    }

    #[test]
    fn the_token_page_is_on_the_server_radar_reads() {
        let mut p = crate::demo::sample();
        p.cfg.api = "https://api.github.com".into();
        assert!(token_page(&p).starts_with("https://github.com/settings/personal-access-tokens/new?name=radar"));
        p.cfg.api = "https://git.ankh-morpork.example/api/v3".into();
        assert!(token_page(&p).starts_with("https://git.ankh-morpork.example/settings/"));
    }
}
