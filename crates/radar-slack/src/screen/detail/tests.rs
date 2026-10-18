//! Owner-local fixed state: no Host, production constructor, clock, credential,
//! process, provider, filesystem or frame is needed to prove this projection.

use super::*;
use crate::client::{ChannelInfo, File, Me, Reaction, Shared};
use crate::config::{Channel, Density, Kind};
use crate::model::ChannelData;
use radar_core::Plugin;
use radar_core::view::{Node, PaneId};

const NOW: i64 = radar_core::plugin::DEMO_NOW;
const CHANNEL: &str = "CWATCH1";
const ROOT: &str = "1791200000.000100";
const REPLY: &str = "1791200060.000200";
const WEB: &str = "https://watch.slack.com";

fn fixture() -> SlackPlugin {
    let mut p = SlackPlugin {
        now: NOW,
        ..Default::default()
    };
    p.data.me = Some(Me {
        team_id: "TWATCH1".into(),
        user_id: "UVIMES1".into(),
        team: "Watch".into(),
        url: WEB.into(),
        ..Default::default()
    });
    p.data.users.insert("UVIMES1".into(), "Sam Vimes".into());
    p.data
        .users
        .insert("UCARROT1".into(), "Carrot Ironfoundersson".into());
    p.cfg.channels.push(Channel {
        id: CHANNEL.into(),
        alias: "watch".into(),
        name: "city-watch".into(),
        purpose: String::new(),
        kind: Kind::Team,
        color: "blue".into(),
        watched: true,
        reader: None,
    });
    let parent = Message {
        ts: ROOT.into(),
        user: Some("UVIMES1".into()),
        text: "Watch report <https://watch.example/report|report>".into(),
        reply_count: 1,
        latest_reply: Some(REPLY.into()),
        ..Default::default()
    };
    let reply = Message {
        ts: REPLY.into(),
        thread_ts: Some(ROOT.into()),
        user: Some("UCARROT1".into()),
        text: "All quiet\nSecond line preserved in cozy".into(),
        edited: true,
        shared: vec![Shared {
            author: "Cheery Littlebottom".into(),
            text: "Gate inspection\nComplete".into(),
            url: "https://watch.example/shared".into(),
        }],
        files: vec![File {
            name: "watch.png".into(),
            url: "https://watch.example/file".into(),
            image: true,
            src: "https://watch.example/image".into(),
            size: Some((30, 20)),
        }],
        reactions: vec![Reaction {
            name: "eyes".into(),
            count: 2,
            users: vec!["UVIMES1".into()],
        }],
        ..Default::default()
    };
    p.data.channels.insert(
        CHANNEL.into(),
        ChannelData {
            info: Some(ChannelInfo {
                id: CHANNEL.into(),
                name: "city-watch".into(),
                ..Default::default()
            }),
            messages: vec![parent],
            replies: [(ROOT.into(), vec![reply])].into(),
            ..Default::default()
        },
    );
    select(
        &mut p,
        Item::Reply {
            channel: CHANNEL.into(),
            thread: ROOT.into(),
            ts: REPLY.into(),
        },
    );
    p
}

fn select(p: &mut SlackPlugin, item: Item) {
    let path = vec!["selected".into()];
    p.shown.borrow_mut().insert(path.clone(), item);
    p.tree.borrow_mut().select(path);
}

fn preview(p: &SlackPlugin) -> PaneView {
    let cx = v::ViewCx {
        now: NOW,
        focus: PaneId::Nav,
        width: 80,
        height: 24,
        parts: radar_core::compose::Parts::new(&[], None),
        icon_lead: 0,
        setup: false,
    };
    super::super::neutral::detail_pane(p, &cx)
}

fn content(pane: &PaneView) -> DetailContent {
    DetailContent::of(&pane.body).expect("shared message detail")
}

fn fact<'a>(content: &'a DetailContent, label: &str) -> Option<&'a Text> {
    content
        .facts
        .iter()
        .find(|fact| fact.label == label)
        .map(|fact| &fact.value)
}

fn section<'a>(content: &'a DetailContent, name: &str) -> &'a [Block] {
    content
        .sections
        .iter()
        .find_map(|block| match block {
            Block::Section { title, blocks, .. } if title == name => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            Block::Line { text, .. } => text.words(),
            Block::Section {
                title,
                blocks,
                detail,
                ..
            } => format!(
                "{title} {} {}",
                detail.as_deref().unwrap_or_default(),
                text(blocks)
            ),
            Block::Doc { doc, .. } => radar_core::ui::rich::plain(doc, 200),
            Block::Wrapped { text, prefix, .. } => format!("{}{text}", prefix.text),
            Block::Meter { .. } => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn both_densities_keep_selected_reply_identity_content_and_actual_detail_actions() {
    let mut p = fixture();
    for density in [Density::Cozy, Density::Compact] {
        p.cfg.density = density;
        let pane = preview(&p);
        let content = content(&pane);
        assert_eq!(content.header.path.words(), "Watch / #watch");
        assert_eq!(content.header.title.words(), "All quiet");
        assert_eq!(
            content.header.author.as_ref().unwrap().words(),
            "Carrot Ironfoundersson"
        );
        assert_eq!(
            content.header.age.as_ref().unwrap().words(),
            format!("{} ago", crate::view::age(REPLY, NOW))
        );
        assert!(content.header.branch.is_none());
        assert_eq!(fact(&content, "replies").unwrap().words(), "1");
        assert_eq!(
            fact(&content, "thread").unwrap().words(),
            "reply in Sam Vimes's thread"
        );
        assert_eq!(fact(&content, "edited").unwrap().words(), "yes");
        assert_eq!(fact(&content, "files").unwrap().words(), "1");
        assert_eq!(fact(&content, "shared messages").unwrap().words(), "1");
        assert_eq!(content.pictures.as_deref(), Some("slack"));
        assert!(
            content
                .sections
                .iter()
                .all(|block| matches!(block, Block::Section { .. }))
        );
        let body = section(
            &content,
            if density == Density::Cozy {
                "MESSAGE"
            } else {
                "THREAD"
            },
        );
        let words = text(body);
        for kept in [
            "All quiet",
            "Gate inspection",
            "Complete",
            "watch.png",
            "👀 2",
        ] {
            assert!(words.contains(kept), "{density:?} lost {kept}: {words}");
        }
        if density == Density::Cozy {
            assert!(words.contains("Second line preserved in cozy"));
            assert!(
                matches!(&body[0], Block::Doc { doc, .. } if doc.blocks.iter().any(|block| matches!(block, radar_core::ui::rich::Block::Media(media) if media.src == "https://watch.example/image")))
            );
            assert!(text(section(&content, "REPLIES")).contains("All quiet"));
        } else {
            assert!(words.contains("Watch report"));
            assert!(body.iter().any(|block| matches!(block, Block::Line { text, .. } if text.0.iter().any(|seg| seg.icon == Some(Icon::Unread)))));
            for block in body {
                match block {
                    Block::Line { text, .. } => assert!(!text.words().starts_with("    ")),
                    Block::Wrapped { prefix, .. } => assert_eq!(prefix.text, "│ "),
                    _ => {}
                }
            }
        }
        let links = text(section(&content, "LINKS"));
        for url in [
            "https://watch.example/report",
            "https://watch.example/shared",
            "https://watch.example/file",
        ] {
            assert!(links.contains(url));
        }
        let footer = pane.foot_actions.as_ref().unwrap();
        let canonical = crate::actions(&super::super::rows(&p, Pane::Detail));
        let ids: Vec<_> = footer
            .actions
            .iter()
            .map(|foot| foot.action.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "slack.feed.open_thread",
                "slack.feed.open_in_slack",
                "slack.feed.copy_message_link",
                "slack.feed.mark_read",
                "slack.feed.reply",
                "slack.feed.react"
            ]
        );
        for foot in &footer.actions {
            let offered = canonical
                .iter()
                .find(|action| action.id == foot.action.id)
                .unwrap();
            let mut compact = offered.clone();
            compact.label = foot.action.label.clone();
            compact.mnemonic_at = foot.action.mnemonic_at;
            compact.mnemonic_shift = foot.action.mnemonic_shift;
            assert_eq!(
                foot.action, compact,
                "keys, change classifications and IDs stay canonical"
            );
            assert!(foot.target.is_none() && foot.disabled.is_none());
        }
        assert_eq!(footer.menu.action.key.unwrap().name(), "x");
        assert!(footer.menu.target.is_none());
        let Node::Extension { fallback, .. } = &pane.body else {
            unreachable!()
        };
        let Node::Rich { blocks, pictures } = fallback.as_ref() else {
            panic!("readable fallback")
        };
        assert_eq!(pictures.as_deref(), Some("slack"));
        assert!(text(blocks).contains("Carrot Ironfoundersson"));
        assert!(text(blocks).contains("comment"));
    }
}

#[test]
fn found_dm_without_cached_thread_omits_unknown_counts_and_keeps_bot_and_links() {
    let mut p = fixture();
    p.data.for_you.push(Found {
        channel_id: "DDWARFS1".into(),
        channel_name: "Cheery Littlebottom".into(),
        is_im: true,
        permalink: "https://watch.slack.com/archives/DDWARFS1/p1791200060000200".into(),
        message: Message {
            bot_name: Some("Hex".into()),
            user: None,
            ..p.data.channels[CHANNEL].replies[ROOT][0].clone()
        },
        ..Default::default()
    });
    select(&mut p, Item::Found(0));
    for density in [Density::Cozy, Density::Compact] {
        p.cfg.density = density;
        let pane = preview(&p);
        let content = content(&pane);
        assert_eq!(pane.title, "Hex's message");
        assert_eq!(content.header.path.words(), "Watch / Cheery Littlebottom");
        assert!(
            content
                .header
                .path
                .0
                .iter()
                .any(|seg| seg.icon == Some(Icon::Direct))
        );
        assert_eq!(content.header.author.as_ref().unwrap().words(), "Hex");
        assert!(
            content
                .header
                .author
                .as_ref()
                .unwrap()
                .0
                .iter()
                .any(|seg| seg.icon == Some(Icon::Bot))
        );
        assert!(fact(&content, "replies").is_none());
        assert_eq!(fact(&content, "thread").unwrap().words(), "reply");
        let links = text(section(&content, "LINKS"));
        assert!(
            links.contains("https://watch.example/shared")
                && links.contains("https://watch.example/file")
        );
        assert!(
            !pane
                .foot_actions
                .unwrap()
                .actions
                .iter()
                .any(|foot| foot.action.id == "slack.feed.mark_read")
        );
    }
}

#[test]
fn cached_zero_and_missing_optional_fields_are_distinct_from_unknown_found_total() {
    let mut p = fixture();
    p.data.channels.get_mut(CHANNEL).unwrap().messages = vec![Message {
        ts: ROOT.into(),
        ..Default::default()
    }];
    p.data.channels.get_mut(CHANNEL).unwrap().replies.clear();
    p.data.me.as_mut().unwrap().team.clear();
    p.data.me.as_mut().unwrap().url.clear();
    select(
        &mut p,
        Item::Thread {
            channel: CHANNEL.into(),
            ts: ROOT.into(),
        },
    );
    let pane = preview(&p);
    let content = content(&pane);
    assert_eq!(content.header.path.words(), "#watch");
    assert_eq!(content.header.title.words(), "(no text)");
    assert!(content.header.author.is_none() && content.header.branch.is_none());
    assert_eq!(fact(&content, "replies").unwrap().words(), "0");
    assert!(fact(&content, "edited").is_none() && fact(&content, "files").is_none());
    assert!(
        !pane
            .foot_actions
            .unwrap()
            .actions
            .iter()
            .any(|foot| matches!(
                foot.action.id.as_str(),
                "slack.feed.open_in_slack" | "slack.feed.copy_message_link"
            ))
    );
    assert!(
        content
            .facts
            .iter()
            .all(|fact| !fact.value.words().trim().is_empty())
    );
}

#[test]
fn found_reply_newer_than_cached_thread_stays_in_compact_body_and_link_list() {
    let mut p = fixture();
    p.data.for_you.push(Found {
        channel_id: CHANNEL.into(),
        message: Message {
            ts: "1791200120.000300".into(),
            thread_ts: Some(ROOT.into()),
            user: Some("UCARROT1".into()),
            text: "New gate report <https://watch.example/new|new link>".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    select(&mut p, Item::Found(0));
    let content = content(&preview(&p));
    assert_eq!(content.header.title.words(), "New gate report new link");
    let body = text(section(&content, "THREAD"));
    assert!(
        body.contains("Watch report")
            && body.contains("All quiet")
            && body.contains("New gate report")
    );
    assert!(text(section(&content, "LINKS")).contains("https://watch.example/new"));
}

#[test]
fn compact_read_markers_track_existing_local_marks() {
    let mut p = fixture();
    p.data.read_to(CHANNEL, ROOT, REPLY);
    let content = content(&preview(&p));
    assert!(!section(&content, "THREAD").iter().any(|block| matches!(block, Block::Line { text, .. } if text.0.iter().any(|seg| seg.icon == Some(Icon::Unread)))));
}

#[test]
fn identity_uses_resolved_subject_account_incarnation_and_destination_never_found_index() {
    let mut p = fixture();
    let original = p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap();
    assert_eq!(original.subject[4..7], [CHANNEL, REPLY, ROOT]);
    p.cfg.density = Density::Cozy;
    assert_eq!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(original.clone())
    );
    p.data.for_you.push(Found {
        channel_id: CHANNEL.into(),
        message: p.data.channels[CHANNEL].replies[ROOT][0].clone(),
        ..Default::default()
    });
    select(&mut p, Item::Found(0));
    assert_eq!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(original.clone())
    );
    p.data.for_you.insert(0, Found::default());
    select(&mut p, Item::Found(1));
    assert_eq!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(original.clone())
    );
    p.data.for_you[1].permalink = "https://watch.example/replacement".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(original.clone())
    );
    p.data.for_you[1].permalink.clear();
    p.data.me.as_mut().unwrap().user_id = "UCARROT1".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(original.clone())
    );
    p.data.me.as_mut().unwrap().user_id = "UVIMES1".into();
    p.invalidate_reads();
    select(&mut p, Item::Found(1));
    let newer = p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap();
    assert_eq!(newer.subject, original.subject);
    assert_ne!(newer.connection, original.connection);
    p.data.for_you[1].message.ts = "1791200120.000300".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail),
        Some(newer)
    );
}

#[test]
fn no_identity_or_footer_on_pages_missing_messages_subjects_headings_alerts_or_empty() {
    let mut p = fixture();
    assert!(p.detail_identity("connection", Pane::Detail).is_none());
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Nav).is_none());
    p.pages.push(crate::page::Page::Thread {
        channel: CHANNEL.into(),
        ts: ROOT.into(),
        tab: 0,
        row: 0,
    });
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    p.pages.clear();
    let subject = radar_core::Subject::new("watch", "WATCH-12", "Gate inspection")
        .with("title", "Inspect the gate")
        .with_url("https://watch.example/subject");
    for item in [
        Item::Heading("City watch".into()),
        Item::Subject(subject),
        Item::Alert {
            channel: CHANNEL.into(),
            ts: "missing".into(),
        },
        Item::Thread {
            channel: CHANNEL.into(),
            ts: "missing".into(),
        },
        Item::Reply {
            channel: CHANNEL.into(),
            thread: ROOT.into(),
            ts: "missing".into(),
        },
        Item::Found(9),
    ] {
        select(&mut p, item.clone());
        assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
        let pane = preview(&p);
        assert!(pane.foot_actions.is_none());
        let Node::Rich { blocks, .. } = &pane.body else {
            panic!("ordinary branch: {item:?}")
        };
        match item {
            Item::Heading(_) => assert!(text(blocks).contains("City watch")),
            Item::Subject(_) => {
                let words = text(blocks);
                for kept in [
                    "Gate inspection",
                    "Inspect the gate",
                    "https://watch.example/subject",
                ] {
                    assert!(words.contains(kept));
                }
            }
            _ => assert!(blocks.is_empty()),
        }
    }
    p.shown.borrow_mut().clear();
    let pane = preview(&p);
    assert!(pane.foot_actions.is_none());
    let Node::Rich { blocks, .. } = pane.body else {
        panic!("ordinary empty branch")
    };
    assert_eq!(text(&blocks), "Nothing selected.");
    select(
        &mut p,
        Item::Reply {
            channel: CHANNEL.into(),
            thread: ROOT.into(),
            ts: REPLY.into(),
        },
    );
    p.data.me = None;
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
}

#[test]
fn alert_states_and_mapping_guidance_keep_their_existing_ordinary_detail() {
    use crate::alerts::{Alert, Collected, Place, State};
    let mut p = fixture();
    select(
        &mut p,
        Item::Alert {
            channel: CHANNEL.into(),
            ts: ROOT.into(),
        },
    );
    for (state, unknown, mapped, expected) in [
        (State::Firing, false, true, "firing for"),
        (State::Resolved, false, true, "resolved after"),
        (
            State::Resolved,
            true,
            false,
            "its firing is before what radar has read",
        ),
        (State::Failed, false, false, "failed"),
    ] {
        p.collected.replace(Some(Collected {
            alerts: vec![Alert {
                name: "Gate alarm".into(),
                state,
                environment: "night-watch".into(),
                priority: "P1".into(),
                summary: "Gate needs inspection".into(),
                link: Some("https://watch.example/alarm".into()),
                labels: Default::default(),
                source: crate::config::Reader::Status,
                channel: CHANNEL.into(),
                ts: ROOT.into(),
                last_ts: REPLY.into(),
                since: NOW - 300,
                ended: (state == State::Resolved).then_some(NOW - 60),
                start_unknown: unknown,
                notices: 2,
                place: if mapped {
                    Place {
                        repo: Some("ankh-morpork/watch/gates".into()),
                        service: "watch".into(),
                    }
                } else {
                    Place::default()
                },
            }],
            ..Default::default()
        }));
        let pane = preview(&p);
        assert!(pane.foot_actions.is_none());
        assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
        let Node::Rich { blocks, .. } = pane.body else {
            panic!("alert remains ordinary rich content")
        };
        let words = text(&blocks);
        for kept in [
            "Gate alarm",
            expected,
            "night-watch",
            "P1",
            "Gate needs inspection",
            "https://watch.example/alarm",
            "BELONGS TO",
            "1 reply in its thread",
            "notified 2 times",
        ] {
            assert!(words.contains(kept), "{state:?} lost {kept}: {words}");
        }
        assert!(words.contains(if mapped {
            "ankh-morpork/watch/gates"
        } else {
            "unmapped: no rule in settings"
        }));
    }
}
