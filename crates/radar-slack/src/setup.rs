//! Slack's step in setup (`radar setup slack`). Its card is the
//! `connection` section, the rows Settings edits: a token found in the
//! environment, the workspace and the token, who it signs in as, the
//! check, the scopes the token has against what radar needs, and the
//! channels watched. Its sheet is the selected row's detail, then the help
//! that row needs: how to make the user token from radar's app manifest
//! ([`crate::manifest`], `m` copies it), or how to find a channel's link.

use radar_core::plugin::{SetupFinds, SetupGroup, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Seg, Text};

use crate::SlackPlugin;
use crate::manifest;

/// radar's guide to connecting Slack.
const GUIDE: &str = "https://github.com/temet-ai/radar/blob/main/docs/user-guide/connect.md#slack";
/// What each scope allows.
const SCOPES: &str = "https://api.slack.com/scopes";

pub(crate) fn step(p: &SlackPlugin) -> SetupStep {
    let step = SetupStep::new("slack", "Slack", "connection")
        .heading("Connect Slack.", "radar reads Slack as you, so it needs a user token (xoxp-).")
        .icon(Icon::Chat)
        .about("channels, mentions")
        .group(SetupGroup::Service);
    match summary(p) {
        Some(s) => step.summary(s),
        None => step,
    }
}

/// `@vimes · 3 channels`: who the token signs in as, and how many
/// channels are watched.
pub(crate) fn summary(p: &SlackPlugin) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(me) = &p.data.me
        && !me.user.is_empty()
    {
        parts.push(format!("@{}", me.user));
    }
    match p.cfg.channels.iter().filter(|c| c.watched).count() {
        0 => {}
        1 => parts.push("1 channel".into()),
        n => parts.push(format!("{n} channels")),
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Where setup may look, once asked: every `SLACK_…` variable.
pub(crate) fn finds() -> SetupFinds {
    SetupFinds { env: vec!["SLACK_"], files: vec![] }
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

/// How to make a user token from radar's manifest.
pub(crate) fn token_help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Connection), "make a user token", Some("about two minutes, once".into()), vec![
            line("1 Open Slack's new-app page, radar's manifest filled in:"),
            link("api.slack.com / new app", &manifest::new_app_url(), &[("o", "open"), ("m", "copy manifest")]),
            line(&format!("  user scopes only: {}", manifest::scopes().len())),
            line("2 Pick your workspace, Next, Create."),
            line("3 Install to Workspace, then Allow."),
            line("4 OAuth & Permissions shows the User OAuth Token, xoxp-…: paste it into token, or:"),
            command("export SLACK_TOKEN=xoxp-…"),
            line("  then the token is env:SLACK_TOKEN"),
            link("api.slack.com / your apps", manifest::APPS, &[]),
        ]),
        Block::section(Some(Icon::Team), "needs an admin?", None, vec![
            line("If Slack says Request to Install, your admin approves it first."),
            line("A missing scope switches off only its own actions."),
        ]),
        more(),
    ]
}

/// How to find a channel to watch.
pub(crate) fn channels_help() -> Vec<Block> {
    vec![
        Block::section(Some(Icon::Channel), "channel links", None, vec![
            line("In Slack: right-click a channel, Copy, Copy link."),
            line("Paste it, a link to a message in it, its id or #name into add a channel."),
            line("Alias, kind and colour: Settings / Slack / channels."),
        ]),
        more(),
    ]
}

fn more() -> Block {
    Block::section(Some(Icon::Link), "more", None, vec![
        link("Slack: what each scope allows", SCOPES, &[]),
        link("radar guide: connect Slack", GUIDE, &[]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_is_the_connection_and_says_who_and_how_many() {
        let mut p = crate::demo::sample();
        let step = step(&p);
        assert_eq!((step.id, step.section, step.group), ("slack", "connection", SetupGroup::Service));
        let n = p.cfg.channels.iter().filter(|c| c.watched).count();
        assert_eq!(summary(&p), Some(format!("@alice · {n} channels")));
        p.data.me = None;
        p.cfg.channels.clear();
        assert_eq!(summary(&p), None);
        assert_eq!(finds().env, vec!["SLACK_"]);
    }

    /// As a setup step the connection card leaves out api, refresh and read
    /// back; ↓ walks only the rows that are drawn.
    #[test]
    fn the_setup_card_leaves_out_api_refresh_and_read_back_and_the_cursor_skips_nothing_drawn() {
        use crate::settings::{WsRow, key, ws_rows};
        use radar_core::plugin::Plugin;
        use radar_core::view::Key;
use crate::keys::Mods;
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
        let drawn = |p: &SlackPlugin, setup| -> Vec<String> {
            let view = p.section_view("connection", &cx(setup)).expect("the connection section");
            let Node::Tree(tree) = view.body else { panic!("a tree") };
            let words = |r: &radar_core::view::TreeRow| r.name.0.iter().map(|s| s.text.as_str()).collect::<String>();
            // The rows the keys walk: the label over the permissions is not one.
            tree.rows
                .iter()
                .flat_map(|r| {
                    let own = (r.id != "permissions").then(|| words(r));
                    own.into_iter().chain(r.children.iter().map(words))
                })
                .collect()
        };
        let has = |rows: &[String], label: &str| rows.iter().any(|r| r.trim_start().starts_with(label));

        let mut p = crate::demo::sample();
        let settings = drawn(&p, false);
        for label in ["api", "refresh", "read back", "workspace", "token"] {
            assert!(has(&settings, label), "Settings keeps {label}: {settings:#?}");
        }
        let in_settings = ws_rows(&p).len();
        assert_eq!(settings.len(), in_settings);

        let card = drawn(&p, true);
        for label in ["api", "refresh", "read back"] {
            assert!(!has(&card, label), "setup leaves out {label}: {card:#?}");
        }
        for label in ["workspace", "token", "CHANNELS"] {
            assert!(has(&card, label), "setup keeps {label}: {card:#?}");
        }
        let rows = ws_rows(&p);
        assert_eq!(card.len(), rows.len(), "the rows keys walk are the rows drawn");
        assert!(!rows.iter().any(|r| matches!(r, WsRow::Api | WsRow::Refresh | WsRow::Days)));

        // ↓ through the whole card never lands on a hidden row or a heading.
        p.section_rows[0] = 0;
        for _ in 0..rows.len() + 2 {
            key(&mut p, "connection", crate::keys::chord(Key::Down, Mods::NONE));
            let row = rows[p.section_rows[0]];
            assert!(row.selectable(), "{row:?}");
            assert!(!matches!(row, WsRow::Api | WsRow::Refresh | WsRow::Days));
        }

        // Back in Settings the rows come back.
        let again = drawn(&p, false);
        assert!(has(&again, "api") && has(&again, "refresh") && has(&again, "read back"));
        assert_eq!(ws_rows(&p).len(), in_settings);
    }

    /// The card: `m` copies the manifest, `o` opens the new-app page with
    /// it, and channels are added with the channels section's own field.
    #[test]
    fn the_connection_copies_the_manifest_and_adds_channels_with_their_own_field() {
        use radar_core::plugin::{Effect, Outcome};
        use radar_core::view::Key;
use crate::keys::Mods;
        use crate::settings::{WsRow, key, ws_rows};
        let press = |p: &mut SlackPlugin, code| key(p, "connection", crate::keys::chord(code, Mods::NONE));
        let mut p = crate::demo::sample();
        let out = press(&mut p, Key::Char('m'));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.first(), Some(Effect::Copy(m)) if m == &manifest::text())));
        let out = press(&mut p, Key::Char('o'));
        assert!(matches!(&out, Outcome::Handled(fx) if matches!(fx.as_slice(), [Effect::Open(u)] if u.contains("manifest_json="))));
        let rows = ws_rows(&p);
        let add = rows.iter().position(|r| *r == WsRow::ChannelAdd).unwrap();
        assert!(!rows[add - 1].selectable() && !rows[add - 2].selectable(), "the heading and the gap");
        p.section_rows[0] = add;
        press(&mut p, Key::Up);
        assert!(ws_rows(&p)[p.section_rows[0]].selectable(), "the cursor steps over them");
        p.section_rows[0] = add;
        press(&mut p, Key::Enter);
        assert!(matches!(p.edit, Some((crate::Target::ChannelAdd, _))));
        let view = crate::settings::connection_view(&p, 80, 0);
        assert!(format!("{:?}", view.detail_blocks).contains("CHANNEL LINKS"));
    }
}
