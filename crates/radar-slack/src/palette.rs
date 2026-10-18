//! Cached messages navigate through Slack's existing thread subjects. Channels
//! have no Show subject; their names are searchable words on these threads.

use std::collections::BTreeMap;

use radar_core::{palette::PaletteTarget, ui::icon::Icon};

use crate::{SlackPlugin, join};

pub(crate) fn targets(p: &SlackPlugin) -> Vec<PaletteTarget> {
    if !p.authenticated() {
        return vec![];
    }
    let owner = p
        .session_owner
        .as_ref()
        .expect("authenticated account checked above");
    let scope = format!("{}/{}", owner.team_id, owner.user_id);
    let mut targets: BTreeMap<(String, String), PaletteTarget> = BTreeMap::new();
    let mut add = |channel: &str, root: &str, text: &str| {
        let subject = join::thread_subject(p, channel, root);
        if join::thread_target(p, &subject).is_none() {
            return;
        }
        let target = targets
            .entry((channel.into(), root.into()))
            .or_insert_with(|| {
                let mut subject = subject;
                subject.label = format!("{} · {root}", subject.label);
                let mut target = PaletteTarget::scoped(subject, &scope);
                target.words = vec![
                    p.alias(channel),
                    channel.into(),
                    "thread".into(),
                    "message".into(),
                ];
                target.icon = Some(Icon::Thread);
                target
            });
        if !text.is_empty() && !target.words.iter().any(|word| word == text) {
            if target.words.len() == 4 {
                target.subject.label = format!(
                    "#{} · {}",
                    p.alias(channel),
                    text.lines().next().unwrap_or(text)
                );
            }
            target.words.push(text.into());
        }
    };
    for (channel, data) in &p.data.channels {
        for message in &data.messages {
            add(
                channel,
                message.thread_ts.as_deref().unwrap_or(&message.ts),
                &message.text,
            );
        }
        for (root, replies) in &data.replies {
            for message in replies {
                add(channel, root, &message.text);
            }
        }
    }
    for found in &p.data.for_you {
        add(
            &found.channel_id,
            found
                .message
                .thread_ts
                .as_deref()
                .unwrap_or(&found.message.ts),
            &found.message.text,
        );
    }
    for (key, fetched) in &p.threads {
        if let (Some((channel, root)), Some(messages)) =
            (key.split_once('\0'), fetched.value.as_ref())
        {
            for message in messages {
                add(channel, root, &message.text);
            }
        }
    }
    targets.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cache::AccountOwner,
        client::{Found, Me, Message},
        model::ChannelData,
    };
    use radar_core::{Plugin, Showing};

    fn plugin() -> SlackPlugin {
        assert!(crate::config::is_channel_id("CWATCH001"));
        let me = Me {
            team_id: "TWATCH".into(),
            user_id: "UVIMES".into(),
            url: "https://watch.slack.com/".into(),
            ..Default::default()
        };
        let mut p = SlackPlugin {
            session_owner: AccountOwner::of(&me),
            ..SlackPlugin::default()
        };
        p.data.me = Some(me);
        p
    }

    #[test]
    fn cached_messages_and_replies_share_one_team_scoped_thread_target() {
        let mut p = plugin();
        let root = Message {
            ts: "1700000000.000012".into(),
            text: "Repair the Watch bell".into(),
            ..Default::default()
        };
        let reply = Message {
            ts: "1700000001.000012".into(),
            thread_ts: Some(root.ts.clone()),
            text: "Vimes will inspect it".into(),
            ..Default::default()
        };
        p.data.channels.insert(
            "CWATCH001".into(),
            ChannelData {
                messages: vec![root.clone()],
                replies: [(root.ts.clone(), vec![reply.clone()])].into(),
                ..Default::default()
            },
        );
        p.data.for_you.push(Found {
            channel_id: "CWATCH001".into(),
            message: reply,
            ..Default::default()
        });
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 1);
        let target = &targets[0];
        assert_eq!(target.subject.id, "CWATCH001/1700000000.000012");
        assert_eq!(target.subject.field("team_id"), Some("TWATCH"));
        assert!(target.words.contains(&root.text));
        assert!(target.words.contains(&"Vimes will inspect it".into()));
        assert_eq!(
            join::thread_target(&p, &target.subject),
            Some(("CWATCH001", root.ts.as_str()))
        );
        let Showing::Page { screen, effects } = p.show(&target.subject) else {
            panic!("cached thread opens")
        };
        assert_eq!(screen, crate::SCREEN.id);
        assert!(!effects.iter().any(|effect| matches!(
            effect,
            radar_core::Effect::Spawn(_) | radar_core::Effect::SpawnAction { .. }
        )));
        assert_eq!(
            p.subject(screen, radar_core::Pane::Middle).unwrap().id,
            target.subject.id
        );
    }

    #[test]
    fn unauthenticated_cached_data_and_empty_channels_are_not_destinations() {
        let mut p = plugin();
        p.data
            .channels
            .insert("CWATCH001".into(), ChannelData::default());
        assert!(p.palette_targets().is_empty());
        p.data
            .channels
            .get_mut("CWATCH001")
            .unwrap()
            .messages
            .push(Message {
                ts: "1700000000.000012".into(),
                ..Default::default()
            });
        assert_eq!(p.palette_targets().len(), 1);
        p.session_owner = None;
        assert!(p.palette_targets().is_empty());
    }
    #[test]
    fn palette_identity_survives_message_edits_but_is_account_scoped() {
        let mut p = plugin();
        p.data.channels.insert(
            "CWATCH001".into(),
            ChannelData {
                messages: vec![Message {
                    ts: "1700000000.000012".into(),
                    text: "Repair the Watch bell".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        let before = p.palette_targets().remove(0).identity;
        p.data.channels.get_mut("CWATCH001").unwrap().messages[0].text =
            "Inspect the Watch bell".into();
        assert_eq!(p.palette_targets()[0].identity, before);
        let me = p.data.me.as_mut().unwrap();
        me.user_id = "UCARROT".into();
        p.session_owner = AccountOwner::of(me);
        assert_ne!(p.palette_targets()[0].identity, before);
        let account = p.palette_targets().remove(0).identity;
        let me = p.data.me.as_mut().unwrap();
        me.team_id = "TLANCRE".into();
        me.url = "https://lancre.slack.com/".into();
        p.session_owner = AccountOwner::of(me);
        assert_ne!(p.palette_targets()[0].identity, account);
    }
}
