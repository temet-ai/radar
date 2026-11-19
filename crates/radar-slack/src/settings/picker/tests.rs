use super::*;
use crate::client::{Recorded, Reply, Request};
use crate::demo::{SVC, TEAM, TRY, sample};
use crate::keys::Mods;
use crate::settings::key;
use crate::{Checked, Target};
use radar_core::Plugin;
use radar_core::view::{Key, Node, PaneId, ViewCx};

fn info(id: &str, name: &str, private: bool, purpose: &str) -> ChannelInfo {
    ChannelInfo {
        id: id.into(),
        name: name.into(),
        is_private: private,
        is_member: true,
        purpose: purpose.into(),
        ..ChannelInfo::default()
    }
}

/// What Slack lists for the sample's token: three the registry holds (two
/// watched, one not) and two it does not.
fn listed() -> Vec<ChannelInfo> {
    vec![
        info("C0GUILD01", "assassins-guild", false, "Quiet work, by arrangement"),
        info("C0DUTY001", "night-duty", true, ""),
        info(SVC, "ankh-post", false, "The Post Office"),
        info(TEAM, "watch-house", false, "Where the Watch keeps its books"),
        info(TRY, "dibbler-snacks", false, ""),
    ]
}

fn ready() -> SlackPlugin {
    let mut p = sample();
    p.joined.channels = listed();
    p.joined.state = State::Ready;
    p
}

/// A section as the words of its rows, in the order the tree draws them.
fn words(p: &SlackPlugin, section: &str) -> Vec<String> {
    let cx = ViewCx {
        now: 0,
        focus: PaneId::Middle,
        width: 100,
        height: 60,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = p.section_view(section, &cx).expect("the section");
    let Node::Tree(tree) = view.body else { panic!("a tree") };
    tree.visible()
        .into_iter()
        .map(|(_, row)| {
            let cells: String = row.cells.iter().map(|c| format!("  {}", c.text.words())).collect();
            format!("{}{cells}", row.name.words())
        })
        .collect()
}

fn text(p: &SlackPlugin, section: &str) -> String {
    words(p, section).join("\n")
}

fn press(p: &mut SlackPlugin, section: &str, code: Key) {
    key(p, section, crate::keys::chord(code, Mods::NONE));
}

/// Put the channels section's cursor on a listed channel.
fn select(p: &mut SlackPlugin, section: &str, name: &str) {
    let k = p.joined.channels.iter().position(|c| c.name == name).unwrap();
    let at = if section == "channels" {
        ch_rows(p).iter().position(|r| *r == ChRow::Pick(PickRow::Channel(k)))
    } else {
        ws_rows(p).iter().position(|r| *r == WsRow::Pick(PickRow::Channel(k)))
    };
    p.section_rows[if section == "channels" { 1 } else { 0 }] = at.expect("the channel is drawn");
}

fn recorded(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, std::sync::Arc<Recorded>) {
    let rec = Recorded::new(replies);
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    (p, rec)
}

fn page(rows: &[ChannelInfo]) -> anyhow::Result<Reply> {
    let rows: Vec<_> = rows
        .iter()
        .map(|c| serde_json::json!({"id": c.id, "name": c.name, "is_private": c.is_private,
            "purpose": {"value": c.purpose}}))
        .collect();
    Ok(Reply {
        status: 200,
        body: serde_json::json!({"ok": true, "channels": rows, "response_metadata": {"next_cursor": ""}}).to_string(),
        ..Reply::default()
    })
}

fn refused(error: &str, needed: &str) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: serde_json::json!({"ok": false, "error": error, "needed": needed}).to_string(),
        ..Reply::default()
    })
}

/// Run the list's task and hand its answer back to the part.
async fn answered(p: &mut SlackPlugin, fx: Vec<Effect>) -> Vec<Effect> {
    let task = fx
        .into_iter()
        .find_map(|e| match e {
            Effect::SpawnAction { task, .. } => Some(task),
            _ => None,
        })
        .expect("the list is asked for in a task");
    p.on_msg(task.await)
}

fn types(rec: &Recorded) -> Vec<String> {
    rec.requests()
        .iter()
        .map(|r: &Request| r.params.iter().find(|(k, _)| k == "types").map(|(_, v)| v.clone()).unwrap_or_default())
        .collect()
}

#[test]
fn nothing_is_drawn_until_the_list_was_asked_for() {
    let p = sample();
    for section in ["connection", "channels"] {
        assert!(!text(&p, section).contains("YOUR CHANNELS"), "{section}");
    }
    assert!(bulk_has_pick(&p));
}

fn bulk_has_pick(p: &SlackPlugin) -> bool {
    crate::settings::bulk(p, "channels").iter().any(|i| i.value == "joined-list")
        && crate::settings::bulk(p, "connection").iter().any(|i| i.value == "joined-list")
}

#[tokio::test]
async fn a_good_check_asks_for_the_list_and_the_list_is_drawn_in_both_sections() {
    let (mut p, rec) = recorded(vec![page(&listed())]);
    let me = p.data.me.clone().unwrap();
    let fx = crate::settings::on_checked(&mut p, "workspace".into(), Ok(Checked::Me(me)));
    assert_eq!(p.joined.state, State::Loading);
    assert!(text(&p, "channels").contains("asking Slack for the channels you are in"));
    answered(&mut p, fx).await;
    assert_eq!(p.joined.state, State::Ready);
    assert_eq!(types(&rec), ["public_channel,private_channel"]);
    for section in ["connection", "channels"] {
        let shown = text(&p, section);
        assert!(shown.contains("YOUR CHANNELS"), "{section}: {shown}");
        assert!(shown.contains("5 channels you are in · 2 watched"), "{section}: {shown}");
        assert!(shown.contains("filter"), "{section}: {shown}");
        for name in ["#assassins-guild", "#night-duty", "#ankh-post", "#watch-house", "#dibbler-snacks"] {
            assert!(shown.contains(name), "{section}: {name}: {shown}");
        }
        assert!(shown.contains("Where the Watch keeps its books"), "{section}: {shown}");
    }
}

#[test]
fn the_tick_is_a_mark_with_icons_off() {
    use radar_core::ui::icon as packs;
    struct Restore(packs::Icons);
    impl Drop for Restore {
        fn drop(&mut self) {
            packs::set_current(&self.0);
        }
    }
    let _restore = Restore(packs::current());
    let (icons, warnings) = packs::resolve("off", &[], &Default::default());
    assert!(warnings.is_empty());
    packs::set_current(&icons);
    let p = ready();
    let rows = words(&p, "channels");
    let row = |name: &str| rows.iter().find(|r| r.contains(name)).unwrap_or_else(|| panic!("{name}: {rows:?}")).trim_start().to_string();
    assert!(row("#watch-house").starts_with("[x] "), "{rows:?}");
    assert!(row("#ankh-post").starts_with("[x] "), "{rows:?}");
    assert!(row("#dibbler-snacks").starts_with("[ ] "), "a channel in the registry that is not watched: {rows:?}");
    assert!(row("#assassins-guild").starts_with("[ ] "), "{rows:?}");
}

#[test]
fn ticking_adds_a_team_channel_and_unticking_keeps_it_unwatched() {
    for section in ["channels", "connection"] {
        let mut p = ready();
        let before = p.cfg.channels.len();
        select(&mut p, section, "assassins-guild");
        press(&mut p, section, Key::Char(' '));
        assert_eq!(p.cfg.channels.len(), before + 1, "{section}");
        let added = p.cfg.channels.last().unwrap();
        assert_eq!((added.id.as_str(), added.name.as_str(), added.watched), ("C0GUILD01", "assassins-guild", true));
        assert_eq!(added.kind, crate::config::Kind::Team);
        assert_eq!(added.alias, "assassins-guild");
        assert!(text(&p, section).contains("5 channels you are in · 3 watched"), "{section}");
        // The cursor stays on the channel although the registry above it grew.
        let at = if section == "channels" { p.section_rows[1] } else { p.section_rows[0] };
        let on = if section == "channels" {
            ch_rows(&p)[at]
        } else {
            match ws_rows(&p)[at] {
                WsRow::Pick(r) => ChRow::Pick(r),
                other => panic!("{other:?}"),
            }
        };
        let k = p.joined.channels.iter().position(|c| c.id == "C0GUILD01").unwrap();
        assert_eq!(on, ChRow::Pick(PickRow::Channel(k)), "{section}");

        press(&mut p, section, Key::Enter);
        assert_eq!(p.cfg.channels.len(), before + 1, "unticking keeps it in the registry");
        assert!(!p.cfg.channels.last().unwrap().watched);
        press(&mut p, section, Key::Enter);
        assert!(p.cfg.channels.last().unwrap().watched, "ticking it again watches it");
        assert_eq!(p.cfg.channels.len(), before + 1);
    }
}

#[test]
fn the_filter_narrows_by_name_and_purpose_and_clears() {
    let mut p = ready();
    let filter = rows(&p).iter().position(|r| *r == PickRow::Filter).unwrap();
    let at = ch_rows(&p).iter().position(|r| *r == ChRow::Pick(PickRow::Filter)).unwrap();
    assert!(filter > 0);
    p.section_rows[1] = at;
    press(&mut p, "channels", Key::Enter);
    assert!(matches!(p.edit, Some((Target::JoinedFilter, _))));
    for c in "POST".chars() {
        press(&mut p, "channels", Key::Char(c));
    }
    press(&mut p, "channels", Key::Enter);
    assert!(p.edit.is_none());
    assert_eq!(p.joined.filter, "POST");
    let shown = text(&p, "channels");
    assert!(shown.contains("#ankh-post") && !shown.contains("#watch-house"), "{shown}");
    assert!(shown.contains("1 of 5"), "{shown}");
    // Every word must be there, in the name or the purpose.
    p.joined.filter = "watch books".into();
    assert_eq!(visible(&p), [3]);
    p.joined.filter = "no such place".into();
    assert!(text(&p, "channels").contains("none match the filter"));
    // c clears it.
    p.section_rows[1] = ch_rows(&p).iter().position(|r| *r == ChRow::Pick(PickRow::Filter)).unwrap();
    press(&mut p, "channels", Key::Char('c'));
    assert!(p.joined.filter.is_empty());
    assert!(text(&p, "channels").contains("#watch-house"));
}

#[test]
fn a_long_list_shows_its_first_rows_and_says_how_many_there_are() {
    let mut p = sample();
    p.joined.channels = (0..250).map(|n| info(&format!("C{n:08}"), &format!("watch-{n:03}"), false, "")).collect();
    p.joined.state = State::Ready;
    assert_eq!(rows(&p).len(), 3 + MAX_SHOWN);
    assert!(text(&p, "channels").contains("first 200 of 250"));
    p.joined.filter = "watch-24".into();
    assert_eq!(rows(&p).len(), 3 + 10);
}

#[tokio::test]
async fn without_groups_read_the_public_channels_are_listed_and_the_row_says_why() {
    let (mut p, rec) = recorded(vec![refused("missing_scope", "groups:read"), page(&listed()[..1])]);
    let fx = load(&mut p);
    answered(&mut p, fx).await;
    assert_eq!(types(&rec), ["public_channel,private_channel", "public_channel"]);
    assert!(p.joined.private_skipped);
    let shown = text(&p, "channels");
    assert!(shown.contains("public ones only: no groups:read"), "{shown}");
    let status = ch_rows(&p).iter().position(|r| *r == ChRow::Pick(PickRow::Status)).unwrap();
    p.section_rows[1] = status;
    let detail = format!("{:?}", detail(&p, PickRow::Status));
    assert!(detail.contains("The token has no groups:read"), "{detail}");
}

#[tokio::test]
async fn a_token_known_to_lack_groups_read_is_not_asked_for_private_channels() {
    let (mut p, rec) = recorded(vec![page(&listed()[..1])]);
    p.data.me.as_mut().unwrap().scopes.retain(|s| s != "groups:read");
    let fx = load(&mut p);
    answered(&mut p, fx).await;
    assert_eq!(types(&rec), ["public_channel"]);
    assert!(p.joined.private_skipped);
}

#[tokio::test]
async fn a_failed_list_says_so_and_enter_asks_again() {
    let (mut p, rec) = recorded(vec![refused("invalid_auth", ""), page(&listed())]);
    let fx = load(&mut p);
    answered(&mut p, fx).await;
    assert!(matches!(p.joined.state, State::Failed(_)));
    let shown = text(&p, "channels");
    assert!(shown.contains("the token was refused"), "{shown}");
    p.section_rows[1] = ch_rows(&p).iter().position(|r| *r == ChRow::Pick(PickRow::Status)).unwrap();
    let fx = crate::settings::key(&mut p, "channels", crate::keys::chord(Key::Enter, Mods::NONE));
    let radar_core::Outcome::Handled(fx) = fx else { panic!("handled") };
    answered(&mut p, fx).await;
    assert_eq!(p.joined.state, State::Ready);
    assert_eq!(rec.requests().len(), 2);
}

#[test]
fn another_token_forgets_the_list() {
    let mut p = ready();
    p.replace_token("env:WATCH_HOUSE_TOKEN".into());
    assert_eq!(p.joined.state, State::NotAsked);
    assert!(p.joined.channels.is_empty());
}

#[test]
fn the_rows_have_their_keys_listed_by_action_id() {
    let mut p = ready();
    select(&mut p, "channels", "night-duty");
    let ids = |p: &SlackPlugin| crate::settings::action_specs(p, "channels").into_iter().map(|a| a.id).collect::<Vec<_>>();
    let ids_now = ids(&p);
    assert!(ids_now.contains(&"slack.channels.toggle_watch".to_string()), "{ids_now:?}");
    assert!(ids_now.contains(&"slack.channels.list".to_string()), "{ids_now:?}");
    p.section_rows[1] = ch_rows(&p).iter().position(|r| *r == ChRow::Pick(PickRow::Filter)).unwrap();
    assert!(ids(&p).contains(&"slack.channels.filter".to_string()));
}
