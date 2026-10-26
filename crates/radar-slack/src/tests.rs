//! The Slack part through the real host: its screen, pages, settings,
//! menus and what it adds to other parts (with stand-ins that know only the
//! shared vocabulary, in `stand_in.rs`).

use radar_core::Host;
use radar_core::host::Event;
use radar_core::view::Key;
use crate::keys::Mods;

use crate::SlackPlugin;
use crate::client::Message;

pub(crate) use crate::demo::*;

#[test]
fn tree_context_selection_requires_a_drawn_row() {
    use radar_core::view::{Intent, Node, PaneId, TreeRow, ViewCx};
    use radar_core::{Outcome, Pane, Plugin};

    let mut p = sample();
    let cx = ViewCx { now: NOW, focus: PaneId::Middle, width: 160, height: 40,
        parts: radar_core::compose::Parts::none(), icon_lead: 2, setup: false };
    let view = p.view(crate::SCREEN.id, &cx).expect("Slack feed view");
    let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("feed tree").body else { panic!("feed tree") };
    let target = tree.visible().get(1).expect("drawn row").0.clone();
    let select = Intent::Context { target: TreeRow::target_of(&target) };
    for _ in 0..2 {
        assert!(matches!(p.intent(crate::SCREEN.id, Pane::Middle, &select), Outcome::Handled(_)));
        assert_eq!(p.tree.borrow().selected(), target);
    }
    let stale = Intent::Context { target: TreeRow::target_of(&["removed".into()]) };
    assert!(matches!(p.intent(crate::SCREEN.id, Pane::Middle, &stale), Outcome::Ignored));
    assert_eq!(p.tree.borrow().selected(), target, "stale targets keep selection");
}

/// The sample, in the real host, on the Slack screen.
pub(crate) fn hosted() -> Host {
    let mut host = Host::new(vec![Box::new(sample())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(sample());
    host
}

/// What shows, as view text (`radar_core::headless::text`): the screen's
/// panes and rows, the open menu and the status line. No painted frame.
pub(crate) fn shown(host: &Host, width: u16, height: u16) -> String {
    radar_core::headless::text_of(host, width, height)
}

/// The open menu or question and the status line under it, as view text:
/// what a menu's snapshot is about, without the screen behind it.
pub(crate) fn menu_shown(host: &Host) -> String {
    radar_core::headless::text::chrome(&host.chrome_view(100, 40))
}

pub(crate) fn press(host: &mut Host, code: Key) -> Vec<radar_core::host::HostEffect> {
    let _ = shown(host, 160, 40);
    host.update(crate::keys::ev(crate::keys::chord(code, Mods::NONE)))
}

/// ↑ on the first row of each pane reaches the top bar; → changes the
/// screen, ↓ comes back to that pane; inside a thread's page ↑ stays.
#[test]
fn up_from_the_top_of_each_pane_reaches_the_bar_and_down_comes_back() {
    use radar_core::host::View;
    for pane in [radar_core::Pane::Nav, radar_core::Pane::Middle, radar_core::Pane::Detail] {
        let mut host = hosted();
        host.focus = pane;
        press(&mut host, Key::Char('g'));
        press(&mut host, Key::Up);
        assert!(host.bar, "{pane:?}: ↑ at the top reaches the bar");
        press(&mut host, Key::Right);
        assert_eq!(host.view, View::Settings);
        press(&mut host, Key::Left);
        press(&mut host, Key::Down);
        assert_eq!((host.view, host.bar, host.focus), (View::Screen(0), false, pane));
    }
    let mut host = hosted();
    on_team(&mut host, 4);
    press(&mut host, Key::Enter);
    assert!(radar_core::Plugin::depth(host.plugins[0].as_ref(), "slack") > 0, "{}", shown(&host, 160, 40));
    press(&mut host, Key::Char('g'));
    press(&mut host, Key::Up);
    assert!(!host.bar, "inside a page");
}

#[test]
fn plus_and_minus_fold_the_feeds_groups_and_minus_on_a_row_closes_its_group() {
    let mut host = hosted();
    host.focus = radar_core::Pane::Middle;
    // SAFETY: the host holds one plugin, a `SlackPlugin` (checked by its id).
    let tree = |host: &Host| {
        assert_eq!(host.plugins[0].id(), "slack");
        let p = unsafe { &*(host.plugins[0].as_ref() as *const dyn radar_core::Plugin as *const SlackPlugin) };
        let t = p.tree.borrow();
        (t.selected().to_vec(), t.open.clone())
    };
    let text = shown(&host, 160, 40);
    assert!(text.split("── status").nth(1).unwrap().contains("+/- expand/collapse"), "{text}");
    for _ in 0..10 {
        if tree(&host).0.len() >= 2 {
            break;
        }
        press(&mut host, Key::Down);
    }
    let row = tree(&host).0;
    assert!(row.len() >= 2, "{row:?}");
    let group = row[..row.len() - 1].to_vec();
    press(&mut host, Key::Char('-'));
    assert_eq!(tree(&host).0, group);
    assert!(!tree(&host).1.contains(&group));
    press(&mut host, Key::Char('+'));
    assert!(tree(&host).1.contains(&group));
    press(&mut host, Key::Char('-'));
    press(&mut host, Key::Char('='));
    assert!(tree(&host).1.contains(&group));
}

pub(crate) fn keys(host: &mut Host, codes: &[Key]) {
    for c in codes {
        press(host, *c);
    }
}

/// To #team's list, its cursor on today's lunch. The rows are: today,
/// lunch, yesterday, the question, Sun 4 Oct, the review ask.
pub(crate) const ON_TEAM: [Key; 4] = [Key::Char('1'), Key::Down, Key::Down, Key::Char('2')];

/// #team's list, the cursor `down` rows below lunch (2: the question, 4:
/// the review ask).
pub(crate) fn on_team(host: &mut Host, down: usize) {
    keys(host, &ON_TEAM);
    keys(host, &vec![Key::Down; down]);
}

pub(crate) fn chars(host: &mut Host, text: &str) {
    for c in text.chars() {
        press(host, Key::Char(c));
    }
}

/// The Slack part next to the stand-ins: a code host (merge requests) and a
/// tracker (tickets). Screens: code `C` (0), tickets `T` (1), Slack `S` (2).
pub(crate) fn hosted_with_parts() -> Host {
    let mut host = Host::new(
        vec![
            Box::new(stand_in::Code::default()),
            Box::new(stand_in::Tracker::default()),
            Box::new(sample()),
        ],
        NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[2] = Box::new(sample());
    host
}

use crate::stand_in;
use radar_core::host::HostEffect;

/// Run the tasks among `fx` (they reach only recorded replies) and hand
/// their answers back to the host, as the runner does.
pub(crate) fn settle(host: &mut Host, fx: Vec<HostEffect>) -> Vec<HostEffect> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut out = Vec::new();
    let mut todo = fx;
    while let Some(e) = todo.pop() {
        match e {
            HostEffect::Spawn { plugin, task } => {
                let msg = rt.block_on(task);
                todo.extend(host.update(Event::Msg { plugin, msg }));
            }
            other => out.push(other),
        }
    }
    out
}

fn ok(body: serde_json::Value) -> anyhow::Result<crate::client::Reply> {
    Ok(crate::client::Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

/// The sample, reaching Slack only through these recorded replies.
fn recorded(replies: Vec<anyhow::Result<crate::client::Reply>>) -> (SlackPlugin, std::sync::Arc<crate::client::Recorded>) {
    let rec = crate::client::Recorded::new(replies);
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    (p, rec)
}

fn hosted_via(p: SlackPlugin) -> Host {
    let mut host = Host::new(vec![Box::new(SlackPlugin::new())], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

fn spawned(fx: &[HostEffect]) -> usize {
    fx.iter().filter(|e| matches!(e, HostEffect::Spawn { .. })).count()
}

/// The demo's review ask of !263 forgotten, so a draft can be started for it:
/// an ask that exists is opened with `s`, not drafted again.
fn forget_the_review_ask(p: &mut SlackPlugin) {
    p.data.asked.clear();
    let group = format!("<!subteam^{}>", crate::demo::GROUP);
    for m in &mut p.data.channels.get_mut(crate::demo::TEAM).expect("the team channel").messages {
        m.text = m.text.replace(&group, "");
    }
}

pub(crate) fn menu_labels(host: &Host) -> Vec<String> {
    host.menu
        .as_ref()
        .map(|m| m.menu.items.iter().map(|i| format!("{} {}", i.keys, i.label)).collect())
        .unwrap_or_default()
}

// --- the screen ---------------------------------------------------------------------------

#[test]
fn for_you_comes_first_with_mentions_messages_and_threads() {
    let host = hosted();
    let text = shown(&host, 160, 40);
    assert!(text.contains("mentions you"), "{text}");
    assert!(text.contains("direct messages"), "{text}");
    assert!(text.contains("threads you are in"), "{text}");
    assert!(text.contains("@Alice Example could you take this one?"), "{text}");
    insta::assert_snapshot!("for_you", text);
}

#[test]
fn a_channel_shows_days_with_threads_folded_under_them() {
    let mut host = hosted();
    keys(&mut host, &[Key::Char('1'), Key::Down, Key::Down]);
    let text = shown(&host, 160, 40);
    assert!(text.contains("6 NEW"), "{text}");
    assert!(text.contains("today") && text.contains("yesterday") && text.contains("Sun 4 Oct"), "{text}");
    assert!(!text.contains("approved, nice one"), "replies are folded: {text}");
    insta::assert_snapshot!("channel_160x40", text);
    // space shows a thread's replies.
    keys(&mut host, &[Key::Char('2'), Key::Down, Key::Down, Key::Down, Key::Down, Key::Char(' ')]);
    let text = shown(&host, 160, 40);
    assert!(text.contains("approved, nice one"), "{text}");
    insta::assert_snapshot!("channel_thread_open_160x40", text);
}

/// Live, a thread started days ago that just got a reply sat below one
/// started yesterday. Lists go by latest activity, newest first, under the
/// day of it, and the row's time is that activity.
#[test]
fn threads_are_ordered_by_their_latest_activity_and_show_its_time() {
    let mut p = sample();
    // The review ask (2 days old) gets a reply 10 minutes ago.
    let ask = ago(2 * 86_400 + 3600);
    let team = p.data.channels.get_mut(TEAM).unwrap();
    team.replies.get_mut(&ask).unwrap().push(msg(&ago(600), CAROL, "merged, thanks"));
    team.messages[0].latest_reply = Some(ago(600));
    team.messages[0].reply_count = 3;
    let mut host = hosted_via(p);
    keys(&mut host, &[Key::Char('1'), Key::Down, Key::Down]);
    let text = shown(&host, 160, 40);
    let row = |needle: &str| text.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle}: {text}"));
    assert!(row("Bob B.  @review-squad :gitlab: mr") < row("Lunch at 12:30?"), "{text}");
    assert!(row("Lunch at 12:30?") < row("Anyone seen ABC-12"), "{text}");
    let ask_row = text.lines().find(|l| l.contains("Bob B.  @review-squad :gitlab: mr")).unwrap();
    assert!(ask_row.contains("10m"), "the time of its newest reply: {ask_row}");
    assert!(row("today  {today}") < row("Bob B.  @review-squad :gitlab: mr"), "under today: {text}");
    // Discussions too.
    keys(&mut host, &[Key::Char('1'), Key::Up]);
    let text = shown(&host, 160, 40);
    let row = |needle: &str| text.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle}: {text}"));
    assert!(row("Bob B.  @review-squad :gitlab: mr") < row("Anyone seen ABC-12"), "{text}");
}

#[test]
fn going_back_to_a_channel_shows_its_days_open_again() {
    let mut host = hosted();
    keys(&mut host, &[Key::Char('1'), Key::Down, Key::Down, Key::Down]);
    keys(&mut host, &[Key::Up]);
    let text = shown(&host, 160, 40);
    assert!(text.contains("Lunch at 12:30?"), "#team's days are open: {text}");
}

#[test]
fn the_draft_names_the_group_as_slack_does_when_settings_have_only_its_id() {
    let mut host = hosted_with_parts();
    let mut p = sample();
    forget_the_review_ask(&mut p);
    p.cfg.review.handle.clear();
    host.plugins[2] = Box::new(p);
    press(&mut host, Key::Char('s'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("mentions @review-squad · the review channel"), "{text}");
}

#[test]
fn discussions_alone_are_all_general_and_name_no_other_part() {
    let mut host = hosted();
    keys(&mut host, &[Key::Char('1'), Key::Down]);
    let text = shown(&host, 160, 40);
    assert!(text.contains("general"), "{text}");
    assert!(text.contains("THE MOST ACTIVE FIRST"), "{text}");
    assert!(!text.contains("BY WHAT THEY ARE ABOUT"), "{text}");
    insta::assert_snapshot!("discussions_alone_160x40", text);
}

#[test]
fn the_preview_scrolls_with_its_own_keys_and_pgdn_from_anywhere() {
    let mut host = hosted();
    on_team(&mut host, 4);
    keys(&mut host, &[Key::Char('3')]);
    let text = shown(&host, 80, 24);
    assert!(text.contains("Add a stats endpoint"), "the review ask: {text}");
    insta::assert_snapshot!("preview_focused_80x24", text);
    // ↑↓ in the focused detail pane move through it, not to the next message.
    let before = host.screen_view(0, crate::SCREEN.id).expect("Slack screen");
    let subject = host.plugins[0].subject(crate::SCREEN.id, radar_core::Pane::Detail)
        .expect("the review ask's thread");
    press(&mut host, Key::Down);
    assert_eq!(host.focus, radar_core::Pane::Detail);
    let after = host.screen_view(0, crate::SCREEN.id).expect("Slack screen");
    assert_eq!(host.plugins[0].subject(crate::SCREEN.id, radar_core::Pane::Detail), Some(subject));
    assert_eq!(after.pane(radar_core::view::PaneId::Middle), before.pane(radar_core::view::PaneId::Middle),
        "scrolling the preview keeps the exact selected tree row and folds");
    assert_eq!(after.pane(radar_core::view::PaneId::Detail), before.pane(radar_core::view::PaneId::Detail),
        "the same message content remains, even when its title scrolls out of view");
    // PgDn from the list scrolls the detail pane.
    press(&mut host, Key::Char('2'));
    let fx = press(&mut host, Key::PageDown);
    assert!(fx.is_empty());
    assert_eq!(host.focus, radar_core::Pane::Middle);
}

#[test]
fn enter_opens_the_threads_page_and_esc_comes_back_to_the_same_row() {
    let mut host = hosted();
    on_team(&mut host, 4);
    let before = shown(&host, 160, 40);
    press(&mut host, Key::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("Slack / #team / Bob Builder: @review-squad"), "{text}");
    assert!(text.contains("approved, nice one"), "{text}");
    insta::assert_snapshot!("thread_page_160x40", text);
    insta::assert_snapshot!("thread_page_80x24", shown(&host, 80, 24));
    press(&mut host, Key::Char(']'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("<:link: Links>"), "{text}");
    assert!(text.contains(MR_263), "a bare link alone: {text}");
    insta::assert_snapshot!("thread_links_160x40", text);
    press(&mut host, Key::Char('x'));
    insta::assert_snapshot!("thread_links_keys", menu_shown(&host));
    press(&mut host, Key::Esc);
    press(&mut host, Key::Esc);
    // Back where it was (the thread now read, so its new marks went).
    let text = shown(&host, 160, 40);
    assert!(before.contains("6 NEW") && text.contains("3 NEW"), "{text}");
    press(&mut host, Key::Char('x'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "Bob Builder's message", "the same row");
}

/// The sample with the review ask's first reply carrying a link, a file and
/// a message shared into it.
pub(crate) fn with_files() -> SlackPlugin {
    let mut p = sample();
    let ask = ago(2 * 86_400 + 3600);
    let reply = &mut p.data.channels.get_mut(TEAM).unwrap().replies.get_mut(&ask).unwrap()[0];
    reply.text = "looking, see <https://docs.example/plan|the plan>".into();
    reply.files = vec![crate::client::File {
        name: "Screenshot.png".into(),
        url: "https://acme.slack.com/files/U1/F1/screenshot.png".into(),
        image: true,
        ..Default::default()
    }];
    reply.shared = vec![crate::client::Shared {
        author: "Carol Coder".into(),
        text: format!("<!subteam^{GROUP}> :gitlab: <{MR_263}|mr> still open"),
        url: format!("https://acme.slack.com/archives/{TEAM}/p1791000000000100"),
    }];
    p
}

/// Live, a thread's screenshots, the message shared into it and the words
/// of its links could not be opened: they are named and linked on the page,
/// and the Links tab lists each by its words, to open.
#[test]
fn a_threads_files_shared_messages_and_links_are_shown_by_name_and_open_from_links() {
    let mut host = hosted_via(with_files());
    on_team(&mut host, 4);
    press(&mut host, Key::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("looking, see the plan"), "the words, not the markup: {text}");
    assert!(!text.contains("<https://"), "{text}");
    assert!(text.contains("│ Carol Coder: @review-squad :gitlab: mr still open"), "one line, cut to fit: {text}");
    assert!(text.contains("▣ Screenshot.png"), "a file by its name: {text}");
    assert!(!text.contains("slack.com/files") && !text.contains("slack.com/archives"), "no raw link: {text}");
    assert!(text.contains("to open: links, files and what it names, on the Links tab"), "{text}");
    insta::assert_snapshot!("thread_files_160x40", text);
    press(&mut host, Key::Char(']'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("the plan  https://docs.example/plan"), "{text}");
    assert!(text.contains("image Screenshot.png"), "{text}");
    insta::assert_snapshot!("thread_files_links_160x40", text);
    // o opens the row's link.
    let rows = host.plugins[0].keys("feed", radar_core::Pane::Middle);
    assert!(rows.iter().any(|b| b.what.contains("open")), "{rows:?}");
    for _ in 0..5 {
        let fx = press(&mut host, Key::Char('o'));
        if matches!(&fx[..], [HostEffect::Open(u)] if u == "https://acme.slack.com/files/U1/F1/screenshot.png") {
            return;
        }
        press(&mut host, Key::Down);
    }
    panic!("the screenshot's link was never opened");
}

/// Live: a mention opened still showed as new. Opening it is reading it,
/// in radar only, and it stays read through a read of Slack and a restart.
#[test]
fn a_mention_opened_is_read_and_stays_read_through_refreshes_and_restarts() {
    let mut host = hosted();
    assert!(shown(&host, 160, 40).contains("4 for you"));
    // For you: the first mention (Bob's, in #team), where the cursor starts.
    keys(&mut host, &[Key::Char('2')]);
    let before = shown(&host, 160, 40);
    assert!(before.contains("● #team Bob B.  @Alice Example could you"), "{before}");
    let fx = press(&mut host, Key::Enter);
    assert!(fx.iter().any(|e| matches!(e, HostEffect::WriteCache { .. })), "kept: {fx:?}");
    press(&mut host, Key::Esc);
    let after = shown(&host, 160, 40);
    assert!(!after.contains("● #team Bob B.  @Alice Example could you"), "{after}");
    assert!(after.contains("#team Bob B.  @Alice Example could you"), "{after}");
    assert!(after.contains("3 for you"), "{after}");
    // The cache keeps it: a new start reads it back.
    let cache = host.plugins[0].snapshot().unwrap();
    let mut again = sample();
    radar_core::Plugin::restore(&mut again, cache);
    assert_eq!(crate::screen::for_you_count(&again), 0, "cache is hidden before authentication");
    assert!(again.install_me(sample().data.me.unwrap()));
    assert_eq!(crate::screen::for_you_count(&again), 3);
    // A fresh read of For you brings the same mention back: still read.
    let found = again.data.for_you.clone();
    again.on_read(crate::read::Read::ForYou {
        groups: None, result: Ok(found), names: vec![],
        action: crate::sync_history::for_you_action("T000ACME1"),
        evidence: Default::default(),
    });
    assert_eq!(crate::screen::for_you_count(&again), 3);
}

#[test]
fn caught_up_to_here_moves_the_local_mark_and_what_is_new() {
    let mut host = hosted();
    on_team(&mut host, 2);
    // On yesterday's question: caught up to it.
    let fx = press(&mut host, Key::Char('m'));
    assert!(fx.iter().any(|e| matches!(e, HostEffect::WriteCache { .. })), "{fx:?}");
    assert!(spawned(&fx) == 0, "nothing is sent to Slack");
    let text = shown(&host, 160, 40);
    assert!(text.contains("2 NEW"), "lunch and the late reply stay new: {text}");
    assert!(host.message.as_deref().unwrap_or("").contains("#team caught up to"), "{:?}", host.message);
    // b: the whole channel.
    press(&mut host, Key::Char('b'));
    assert!(menu_labels(&host).iter().any(|l| l.contains("mark #team caught up")), "{:?}", menu_labels(&host));
    press(&mut host, Key::Char('m'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("CAUGHT UP"), "{text}");
}

#[test]
fn o_opens_a_message_in_slack_and_y_copies_its_link() {
    let mut host = hosted();
    on_team(&mut host, 4);
    let fx = press(&mut host, Key::Char('o'));
    let ts = ago(2 * 86_400 + 3600).replace('.', "");
    assert!(
        matches!(&fx[..], [HostEffect::Open(u)] if *u == format!("https://acme.slack.com/archives/{TEAM}/p{ts}")),
        "{fx:?}"
    );
    let fx = press(&mut host, Key::Char('y'));
    assert!(matches!(&fx[..], [HostEffect::Copy(u)] if u.contains("/archives/")), "{fx:?}");
}

/// With the Slack app running here, o opens the message in it (its own
/// slack:// link), with the web link for when the app does not take it.
#[test]
fn o_opens_a_message_in_the_slack_app_when_it_runs_here() {
    let mut p = sample();
    p.app_running = Some(true);
    let mut host = hosted_via(p);
    on_team(&mut host, 4);
    let fx = press(&mut host, Key::Char('o'));
    let ts = ago(2 * 86_400 + 3600);
    let web = format!("https://acme.slack.com/archives/{TEAM}/p{}", ts.replace('.', ""));
    let app = format!("slack://channel?team=T000ACME1&id={TEAM}&message={ts}");
    assert!(
        matches!(&fx[..], [HostEffect::OpenApp { link, web: w }] if *link == app && *w == web),
        "{fx:?}"
    );
    // A reply names its thread; the thread page opens the same way.
    press(&mut host, Key::Enter);
    let fx = press(&mut host, Key::Char('o'));
    assert!(matches!(&fx[..], [HostEffect::OpenApp { link, .. }] if *link == app), "{fx:?}");
    assert_eq!(
        crate::app_link("T1", "C1", "2.0", Some("1.0")),
        "slack://channel?team=T1&id=C1&message=2.0&thread_ts=1.0"
    );
    // Not known whether it runs: the machine is asked first, in a task.
    let mut p = sample();
    p.app_running = None;
    let fx = p.open_in_slack(TEAM, "1.0", None, "https://acme.slack.com/archives/x".into());
    assert!(matches!(&fx[..], [radar_core::plugin::Effect::Spawn(_)]), "{fx:?}");
    // Not signed in yet: no team to name, so the web link.
    p.data.me = None;
    let fx = p.open_in_slack(TEAM, "1.0", None, "https://acme.slack.com/archives/x".into());
    assert!(matches!(&fx[..], [radar_core::plugin::Effect::Open(_)]), "{fx:?}");
}

#[test]
fn the_rail_answers_a_selection() {
    use radar_core::view::{Intent, Target};
    let mut host = hosted();
    // The nav pane's #team row, by the target the view gives it.
    let text = shown(&host, 160, 40);
    assert!(text.contains("#team  7 NEW  {rail:2}") || text.contains("#team  6 NEW  {rail:2}"), "{text}");
    host.update(Event::Intent(Intent::Select { target: Target::new("rail:2") }));
    assert_eq!(host.focus, radar_core::Pane::Nav, "selecting a rail row focuses the nav pane");
    let text = shown(&host, 160, 40);
    assert!(text.contains("6 NEW"), "{text}");
}

#[test]
fn the_x_menu_of_a_message_lists_what_works_there() {
    let mut host = hosted();
    on_team(&mut host, 4);
    press(&mut host, Key::Char('x'));
    let labels = menu_labels(&host);
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "Bob Builder's message");
    for want in ["⏎ open its thread", "o open it in Slack", "y copy its link", "m mark read up to here", "c comment in its thread", "e react to it"] {
        assert!(labels.iter().any(|l| l.starts_with(want)), "{want}: {labels:?}");
    }
    insta::assert_snapshot!("message_keys", menu_shown(&host));
    // Refreshing is the pane's: `x` again, inside the row menu.
    press(&mut host, Key::Char('x'));
    let labels = menu_labels(&host);
    assert!(labels.iter().any(|l| l.starts_with("r refresh")), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("comment in its thread")), "{labels:?}");
}

// --- not set up, and set up from the environment --------------------------------------------

#[test]
fn not_set_up_it_says_what_it_needs_and_names_a_variable_it_found_never_its_value() {
    let mut p = SlackPlugin::new();
    p.env = Some(vec![
        ("SLACK_BOT_TOKEN".into(), "xoxb-123".into()),
        ("SLACK_APP_USER_OAUTH_TOKEN".into(), "xoxp-secret-value-1234567890".into()),
    ]);
    assert!(radar_core::Plugin::first_run(&mut p).unwrap().contains("SLACK_APP_USER_OAUTH_TOKEN"));
    let mut host = Host::new(vec![Box::new(p)], NOW).unwrap();
    host.load_config(Some("")).unwrap();
    // The env the plugin looks in is the test's (load_config keeps it).
    let text = shown(&host, 120, 30);
    assert!(text.contains("Slack is not set up yet"), "{text}");
    assert!(text.contains("SLACK_APP_USER_OAUTH_TOKEN"), "{text}");
    assert!(!text.contains("secret-value"), "{text}");
    assert!(!text.contains("SLACK_BOT_TOKEN"), "bot tokens are not offered: {text}");
    insta::assert_snapshot!("set_up_120x30", text);
    // ⏎ goes to connection, where the variable is asked about; ⏎ uses it.
    press(&mut host, Key::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("? Found in your environment: a Slack user token (SLACK_APP_USER_OAUTH_TOKEN). Use it?"), "{text}");
    assert!(text.contains("shows the values or saves them"), "{text}");
    insta::assert_snapshot!("workspace_offer_160x40", text);
    press(&mut host, Key::Enter);
    let fx = press(&mut host, Key::Enter);
    let _ = fx;
    let cfg = host.config_text();
    assert!(cfg.contains("token = \"env:SLACK_APP_USER_OAUTH_TOKEN\""), "{cfg}");
    assert!(!cfg.contains("secret-value"), "{cfg}");
}

#[test]
fn set_up_with_no_channels_it_says_how_to_add_one() {
    let mut p = sample();
    p.cfg.channels.clear();
    p.data.for_you.clear();
    let host = hosted_via(p);
    let text = shown(&host, 120, 30);
    assert!(text.contains("No channels yet"), "{text}");
    insta::assert_snapshot!("no_channels_120x30", text);
}

// --- settings --------------------------------------------------------------------------------

fn settings_at(section: usize) -> Host {
    let mut host = hosted();
    press(&mut host, Key::Char(radar_core::host::SETTINGS_KEY));
    let at = host
        .sections
        .iter()
        .position(|(_, s)| s.group == "slack")
        .unwrap();
    host.section = at + section;
    host.focus = radar_core::Pane::Middle;
    host
}

#[test]
fn each_settings_section_reads_as_view_data() {
    for (i, name) in ["connection", "channels", "review", "alerts", "look"].iter().enumerate() {
        let host = settings_at(i);
        insta::assert_snapshot!(format!("settings_{name}_160x40"), shown(&host, 160, 40));
        // The parts fit some words to the room they are asked for; where
        // that changes nothing, one snapshot says it all.
        if *name != "alerts" {
            insta::assert_snapshot!(format!("settings_{name}_80x24"), shown(&host, 80, 24));
        }
    }
}

#[test]
fn the_workspace_shows_who_and_what_the_token_may_do() {
    let host = settings_at(0);
    let text = shown(&host, 160, 40);
    // In the order every part's connection has.
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s}: {text}"));
    let order = [
        "workspace       Acme · https://acme.slack.com",
        "token           xoxp…ests (typed in)",
        "signed in as    Alice Example  alice",
        "check           not checked yet",
        "refresh         every 2m",
        "read back       7 days",
        "permissions",
    ]
    .map(at);
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.contains("reacting  needs reactions:write"), "{text}");
    assert!(text.contains("pictures"), "{text}");
    assert!(!text.contains("typed-in-token"), "{text}");
}

#[test]
fn typing_a_token_checks_it_as_you_type_and_refuses_a_bot_token() {
    let mut host = settings_at(0);
    press(&mut host, Key::Down); // the api row, under the workspace
    press(&mut host, Key::Down); // the token row, under the api
    press(&mut host, Key::Char('e'));
    assert!(host.plugins[0].capturing(), "editing");
    let text = shown(&host, 160, 40);
    assert!(!text.contains("typed-in"), "a typed token is not shown again: {text}");
    press(&mut host, Key::Char('u'));
    for _ in 0..40 {
        press(&mut host, Key::Backspace);
    }
    chars(&mut host, "xoxb-12345678901234567890");
    let text = shown(&host, 160, 40);
    assert!(text.contains("radar needs a user token"), "{text}");
    press(&mut host, Key::Enter);
    assert!(host.message.as_deref().unwrap_or("").starts_with("not yet"), "{:?}", host.message);
    for _ in 0..40 {
        press(&mut host, Key::Backspace);
    }
    host.update(Event::Paste("xoxp-12345678901234567890".into()));
    press(&mut host, Key::Enter);
    // Kept, and asked whether to check it.
    assert_eq!(host.menu.as_ref().map(|m| m.menu.title.as_str()), Some("check the token now?"));
}

#[test]
fn adding_a_channel_asks_slack_which_it_is_and_adds_it_watched() {
    let (p, rec) = recorded(vec![ok(serde_json::json!({
        "ok": true,
        "channel": {"id": "C00000NEW1", "name": "acme-deploys", "is_private": false, "is_member": true,
                    "purpose": {"value": "where deploys are announced"}}
    }))]);
    let mut host = hosted_via(p);
    press(&mut host, Key::Char(radar_core::host::SETTINGS_KEY));
    host.section = host.sections.iter().position(|(_, s)| s.id == "channels").unwrap();
    host.focus = radar_core::Pane::Middle;
    press(&mut host, Key::Enter); // + add a channel
    host.update(Event::Paste("https://acme.slack.com/archives/C00000NEW1".into()));
    let text = shown(&host, 160, 40);
    assert!(text.contains("the channel C00000NEW1"), "{text}");
    let fx = press(&mut host, Key::Enter);
    let rest = settle(&mut host, fx);
    let _ = rest;
    let req = &rec.requests()[0];
    assert_eq!(req.method, "conversations.info");
    assert!(req.params.contains(&("channel".into(), "C00000NEW1".into())), "{req:?}");
    let cfg = host.config_text();
    assert!(cfg.contains("alias = \"acme-deploys\""), "its own name, not its last word: {cfg}");
    assert!(cfg.contains("name = \"acme-deploys\""), "{cfg}");
    insta::assert_snapshot!("channel_added_160x40", shown(&host, 160, 40));
}

/// The channels section, focused, on `row` (0 is `+ add a channel`).
fn channels_at(host: &mut Host, row: usize) {
    press(host, Key::Char(radar_core::host::SETTINGS_KEY));
    host.section = host.sections.iter().position(|(_, s)| s.id == "channels").unwrap();
    host.focus = radar_core::Pane::Middle;
    keys(host, &vec![Key::Down; row]);
}

#[test]
fn adding_by_name_finds_a_private_channel_you_are_in_however_far_down_the_list() {
    // Slack's lists come in pages that are often nearly empty; the channel
    // is on the 25th page of the ones you are in.
    let mut replies: Vec<anyhow::Result<crate::client::Reply>> = (0..24)
        .map(|i| ok(serde_json::json!({"ok": true, "channels": [{"id": format!("C0OTHER{i:03}"), "name": "other"}],
            "response_metadata": {"next_cursor": format!("n{i}")}})))
        .collect();
    replies.push(ok(serde_json::json!({"ok": true, "channels": [
        {"id": "C0PRIVATE1", "name": "acme-team-private", "is_private": true}],
        "response_metadata": {"next_cursor": ""}})));
    let (p, rec) = recorded(replies);
    let mut host = hosted_via(p);
    channels_at(&mut host, 0);
    press(&mut host, Key::Enter);
    chars(&mut host, "#acme-team-private");
    let fx = press(&mut host, Key::Enter);
    settle(&mut host, fx);
    let methods: Vec<&str> = rec.requests().iter().map(|r| r.method).collect();
    assert_eq!(methods.iter().filter(|m| **m == "users.conversations").count(), 25, "every page, to the end: {methods:?}");
    assert!(!methods.contains(&"conversations.list"), "found among the channels you are in: {methods:?}");
    let cfg = host.config_text();
    assert!(cfg.contains("alias = \"acme-team-private\""), "{cfg}");
    assert!(cfg.contains("id = \"C0PRIVATE1\""), "{cfg}");
    // users.conversations leaves is_member out: listed there, you are in it.
    let text = shown(&host, 160, 40);
    assert!(text.contains("private · you are in it"), "{text}");
}

#[test]
fn adding_by_name_shows_incomplete_search_and_wrapped_retry_guidance() {
    let (p, rec) = recorded(vec![
        ok(serde_json::json!({"ok": true, "channels": [{"id": "C0000TEAM1", "name": "acme-team"}], "response_metadata": {"next_cursor": ""}})),
        ok(serde_json::json!({"ok": true, "channels": [{"id": "C0000GEN01", "name": "general"}, {"id": "C0000RND01", "name": "random"}], "response_metadata": {"next_cursor": ""}})),
    ]);
    let mut host = hosted_via(p);
    channels_at(&mut host, 0);
    let before = host.config_text();
    press(&mut host, Key::Enter);
    chars(&mut host, "acme-secret");
    let fx = press(&mut host, Key::Enter);
    settle(&mut host, fx);
    assert_eq!(rec.requests().len(), 2);
    assert_eq!(rec.requests()[1].method, "conversations.list", "then the public list");
    let said = host.message.clone().unwrap_or_default();
    // These recorded rows have parser defaults; a complete cursor traversal
    // alone is not sufficient to claim a complete negative search.
    assert!(said.contains("Search for #acme-secret was incomplete"), "{said}");
    assert!(said.contains("a match could not be established"), "{said}");
    assert!(said.contains("Retry with a channel ID or link"), "{said}");
    assert!(!said.contains("not among") && !said.contains("private channel"), "{said}");
    assert_eq!(host.config_text(), before);
    for (width, height) in [(160, 40), (80, 24)] {
        let text = shown(&host, width, height);
        // The section's detail, its lines joined: the guidance reads whole
        // whatever the room it was fitted to.
        let detail = text.split("── detail").nth(1).expect("the section's detail");
        let body = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        for visible in ["Search for", "acme-secret", "incomplete", "channel ID or link"] {
            assert!(body.contains(visible), "the add row's detail wraps at {width}x{height}: {text}");
        }
    }
}

#[test]
fn an_alias_can_be_the_channels_whole_name_and_enter_goes_straight_to_typing_it() {
    let mut host = hosted();
    // #team, then its alias.
    channels_at(&mut host, 2);
    press(&mut host, Key::Enter);
    assert!(host.menu.is_none(), "no 'change it' step");
    assert!(host.plugins[0].capturing(), "typing");
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "int-acme-team-billing-reports");
    press(&mut host, Key::Enter);
    assert!(!host.plugins[0].capturing(), "kept: {:?}", host.message);
    let cfg = host.config_text();
    assert!(cfg.contains("alias = \"int-acme-team-billing-reports\""), "{cfg}");
    assert!(cfg.contains("channel = \"int-acme-team-billing-reports\""), "the review channel follows: {cfg}");
    // Saved with w.
    let fx = press(&mut host, Key::Char('w'));
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Save(t) if t.contains("int-acme-team-billing-reports"))), "{fx:?}");
}

#[test]
fn enter_on_purpose_kind_and_colour_goes_straight_to_typing_or_choosing() {
    let mut host = hosted();
    channels_at(&mut host, 3);
    press(&mut host, Key::Enter);
    assert!(host.plugins[0].capturing(), "purpose: typing at once");
    press(&mut host, Key::Esc);
    press(&mut host, Key::Down);
    press(&mut host, Key::Enter);
    assert_eq!(host.menu.as_ref().map(|m| m.menu.title.as_str()), Some("#team is"), "kind: its choices at once");
    press(&mut host, Key::Esc);
    press(&mut host, Key::Down);
    press(&mut host, Key::Enter);
    assert_eq!(host.menu.as_ref().map(|m| m.menu.title.as_str()), Some("channel colour"));
}

#[test]
fn the_template_previews_as_you_type_and_must_keep_mrs() {
    let mut host = settings_at(2);
    keys(&mut host, &[Key::Down, Key::Down, Key::Enter]);
    keys(&mut host, &[Key::Backspace; 30]);
    chars(&mut host, "Review please {group}: {mrs}");
    let text = shown(&host, 160, 40);
    assert!(text.contains("Review please @review-squad: mr <https://git.example/acme/team-a/api/-/merge_requests/263> Add a stats"), "{text}");
    assert!(text.contains("[Review please {group}: {mrs}|]"), "the input over the row: {text}");
    insta::assert_snapshot!("review_template_typing_160x40", text);
    keys(&mut host, &[Key::Backspace; 40]);
    chars(&mut host, "no placeholder");
    press(&mut host, Key::Enter);
    assert!(host.message.as_deref().unwrap_or("").contains("needs {mrs}"), "{:?}", host.message);
}

fn usergroups_reply() -> anyhow::Result<crate::client::Reply> {
    ok(serde_json::json!({"ok": true, "usergroups": [
        {"id": GROUP, "handle": "review-squad", "name": "Review squad", "users": [ALICE, BOB]},
        {"id": "S0ACMEDEV1", "handle": "acme-devs", "name": "Acme devs", "users": [CAROL]}]}))
}

#[test]
fn the_review_group_is_picked_from_slacks_own_list_stored_by_id_and_shown_by_handle() {
    let (mut p, rec) = recorded(vec![usergroups_reply()]);
    p.cfg.review.group.clear();
    p.cfg.review.handle.clear();
    let mut host = hosted_via(p);
    press(&mut host, Key::Char(radar_core::host::SETTINGS_KEY));
    host.section = host.sections.iter().position(|(_, s)| s.id == "review").unwrap();
    host.focus = radar_core::Pane::Middle;
    press(&mut host, Key::Down);
    // ⏎ asks Slack (usergroups.list) at once, then offers what it has.
    let fx = press(&mut host, Key::Enter);
    settle(&mut host, fx);
    assert_eq!(rec.requests()[0].method, "usergroups.list");
    let menu = &host.menu.as_ref().expect("the groups to pick from").menu;
    assert_eq!(menu.title, "mention which user group?");
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["@review-squad", "@acme-devs"], "yours first");
    host.menu.as_mut().unwrap().row = 1;
    press(&mut host, Key::Enter);
    let cfg = host.config_text();
    assert!(cfg.contains("group = \"S0ACMEDEV1\"") && cfg.contains("handle = \"acme-devs\""), "{cfg}");
    assert!(shown(&host, 160, 40).contains("user group       @acme-devs"));
}

/// The template as it was typed live, `{@handle} :gitlab: {mrs}`: checked
/// against Slack's groups, and the ask sent with Slack's mention of the
/// group's id, which pings.
#[test]
fn a_template_naming_a_group_by_handle_is_checked_and_sends_the_groups_mention() {
    let rec = crate::client::Recorded::new(vec![ok(serde_json::json!({"ok": true, "channel": TEAM, "ts": "1791271000.000500"}))]);
    let mut host = hosted_with_parts();
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    p.cfg.review.group.clear();
    p.cfg.review.handle.clear();
    p.cfg.review.template = "{@acme-devs} :gitlab: {mrs}".into();
    p.data.groups.push(crate::client::UserGroup {
        id: "S0ACMEDEV1".into(),
        handle: "acme-devs".into(),
        name: "Acme devs".into(),
        users: vec![CAROL.into()],
    });
    host.plugins[2] = Box::new(p);
    press(&mut host, Key::Down);
    press(&mut host, Key::Char('s'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("mentions @acme-devs · the review channel"), "{text}");
    assert!(text.contains("@acme-devs :gitlab: mr <https://git.example/acme/team-a/api/-/merge_requests/270> Tidy the README"), "{text}");
    press(&mut host, Key::Char('p'));
    let fx = press(&mut host, Key::Char('y'));
    settle(&mut host, fx);
    let body = rec.requests()[0].body.clone().unwrap();
    assert_eq!(
        body["text"],
        "<!subteam^S0ACMEDEV1> :gitlab: <https://git.example/acme/team-a/api/-/merge_requests/270|mr> Tidy the README"
    );
}

#[test]
fn a_template_slack_would_not_understand_is_refused_as_you_type() {
    let mut host = settings_at(2);
    keys(&mut host, &[Key::Down, Key::Down, Key::Enter]);
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "{@nobody} :gitlab: {mrs}");
    assert!(shown(&host, 160, 40).contains(":error: {@nobody}: Slack has no user group with that"));
    press(&mut host, Key::Enter);
    assert!(host.message.as_deref().unwrap_or("").starts_with("not yet: {@nobody}"), "{:?}", host.message);
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "@review-squad :gitlab: {mrs}");
    press(&mut host, Key::Enter);
    assert!(host.message.as_deref().unwrap_or("").contains("ping no one"), "{:?}", host.message);
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "{@review-squad} :gitlab: {mrs}");
    press(&mut host, Key::Enter);
    assert!(host.config_text().contains("template = \"{@review-squad} :gitlab: {mrs}\""));
}

#[test]
fn when_to_ask_again_is_typed_as_hours_days_or_a_time_and_kept() {
    let mut host = settings_at(2);
    keys(&mut host, &[Key::Down; 5]);
    press(&mut host, Key::Enter);
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "10:00");
    let text = shown(&host, 160, 40);
    assert!(text.contains(":run_passed: offers to ask again at 10:00"), "{text}");
    press(&mut host, Key::Enter);
    assert!(host.config_text().contains("reask = \"10:00\""), "{}", host.config_text());
    assert!(shown(&host, 160, 40).contains("ask again        at 10:00 on the next working day without a reply"));
    press(&mut host, Key::Enter);
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('u'), Mods::CONTROL)));
    chars(&mut host, "3 weeks");
    press(&mut host, Key::Enter);
    assert!(host.message.as_deref().unwrap_or("").starts_with("not yet"), "{:?}", host.message);
}

/// An ask radar posted 3 hours ago with no reply, set to ask again after 2
/// working hours: the merge request offers it, and the reply mentions the
/// group and links to the ask it follows up.
#[test]
fn asking_again_replies_in_the_thread_with_the_group_and_a_link_to_the_ask() {
    let ask_ts = ago(3 * 3600);
    let rec = ByMethod::new(vec![
        ("conversations.replies", ok(serde_json::json!({"ok": true, "messages": [
            {"ts": ask_ts, "user": ALICE, "text": "<!subteam^S0REVIEW01> :gitlab: <https://git.example/acme/team-a/api/-/merge_requests/270|mr> Tidy the README"}]}))),
        ("chat.postMessage", ok(serde_json::json!({"ok": true, "channel": TEAM, "ts": "1791271000.000900"}))),
        ("conversations.replies", ok(serde_json::json!({"ok": true, "messages": []}))),
    ]);
    let mut host = hosted_with_parts();
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    p.cfg.review.reask = Some("2h".into());
    p.data.asked.push(crate::model::Asked {
        mrs: vec![("acme/team-a/api!270".into(), "https://git.example/acme/team-a/api/-/merge_requests/270".into())],
        channel: TEAM.into(),
        ts: ask_ts.clone(),
        ..Default::default()
    });
    host.plugins[2] = Box::new(p);
    press(&mut host, Key::Down);
    let text = shown(&host, 160, 40);
    assert!(text.contains("api !270 Tidy the README [asked 3h · no reply]"), "{text}");
    press(&mut host, Key::Char('x'));
    assert!(menu_labels(&host).iter().any(|l| l.contains("ask again in the thread")), "{:?}", menu_labels(&host));
    press(&mut host, Key::Esc);
    let fx = press(&mut host, Key::Char('n'));
    settle(&mut host, fx);
    let link = format!("https://acme.slack.com/archives/{TEAM}/p{}", ask_ts.replace('.', ""));
    // The words come written in the composer; ctrl-s shows them and asks.
    host.update(crate::keys::ev(crate::keys::chord(Key::Char('s'), Mods::CONTROL)));
    let fx = press(&mut host, Key::Char('y'));
    settle(&mut host, fx);
    let seen = rec.seen.lock().unwrap().clone();
    let post = seen.iter().find(|r| r.method == "chat.postMessage").expect("posted");
    let body = post.body.clone().unwrap();
    assert_eq!(
        body["text"],
        format!("<!subteam^{GROUP}> still looking for a review on <{link}|this review ask>, please.")
    );
    assert_eq!(body["thread_ts"], ask_ts);
}

#[test]
fn the_config_round_trips_through_the_host() {
    let host = hosted();
    let text = host.config_text();
    assert!(text.contains("[[slack.channel]]"), "{text}");
    assert!(text.contains("[slack.review]"), "{text}");
    let mut again = Host::new(vec![Box::new(SlackPlugin::new())], NOW).unwrap();
    again.load_config(Some(&text)).unwrap();
    assert_eq!(again.config_text(), text);
}

// --- reading, against recorded replies ----------------------------------------------------------

/// Recorded replies by method: a read's tasks run in no fixed order.
#[derive(Default)]
pub(crate) struct ByMethod {
    replies: std::sync::Mutex<std::collections::BTreeMap<&'static str, std::collections::VecDeque<anyhow::Result<crate::client::Reply>>>>,
    seen: std::sync::Mutex<Vec<crate::client::Request>>,
}

impl ByMethod {
    fn new(replies: Vec<(&'static str, anyhow::Result<crate::client::Reply>)>) -> std::sync::Arc<Self> {
        let me = Self::default();
        for (m, r) in replies {
            me.replies.lock().unwrap().entry(m).or_default().push_back(r);
        }
        std::sync::Arc::new(me)
    }

    fn methods(&self) -> Vec<&'static str> {
        self.seen.lock().unwrap().iter().map(|r| r.method).collect()
    }
}

impl crate::client::Transport for ByMethod {
    fn send<'a>(&'a self, req: &'a crate::client::Request) -> crate::client::Fut<'a, anyhow::Result<crate::client::Reply>> {
        self.seen.lock().unwrap().push(req.clone());
        let next = self
            .replies
            .lock()
            .unwrap()
            .get_mut(req.method)
            .and_then(|q| q.pop_front())
            .unwrap_or_else(|| Err(anyhow::anyhow!("no recorded reply for {}", req.method)));
        Box::pin(async move { next })
    }
}

#[test]
fn a_read_asks_for_the_channels_threads_for_you_and_names_then_holds_them() {
    let mut p = sample();
    // One watched channel, already known; the groups fresh.
    p.cfg.channels.truncate(1);
    p.data.channels.clear();
    p.data.channels.entry(TEAM.into()).or_default().info = Some(crate::client::ChannelInfo {
        id: TEAM.into(),
        name: "acme-team".into(),
        ..Default::default()
    });
    p.data.for_you.clear();
    p.data.users.remove(CAROL);
    let parent = serde_json::json!({"ts": "1791270000.000100", "user": BOB, "text": format!("ask <{MR_263}|mr>"),
        "reply_count": 1, "reply_users": [CAROL], "latest_reply": "1791270100.000100"});
    let none = || ok(serde_json::json!({"ok": true, "messages": {"matches": [], "paging": {"pages": 1}}}));
    let carol = || ok(serde_json::json!({"ok": true, "user": {"id": CAROL, "name": "carol", "profile": {"display_name": "Carol C"}}}));
    let rec = ByMethod::new(vec![
        ("conversations.history", ok(serde_json::json!({"ok": true, "messages": [parent.clone(), {"ts": "1791269000.000100", "user": BOB, "text": "joined", "subtype": "channel_join"}], "has_more": false}))),
        ("conversations.replies", ok(serde_json::json!({"ok": true, "messages": [parent, {"ts": "1791270100.000100", "user": CAROL, "text": "on it", "thread_ts": "1791270000.000100"}]}))),
        ("search.messages", ok(serde_json::json!({"ok": true, "messages": {"matches": [{"ts": "1791270200.000100", "user": CAROL, "text": format!("<@{ALICE}> hi"), "channel": {"id": "D1", "name": "D1", "is_im": true}, "permalink": "https://acme.slack.com/archives/D1/p1791270200000100"}], "paging": {"pages": 1}}}))),
        ("search.messages", none()),
        ("search.messages", none()),
        ("users.info", carol()),
        ("users.info", carol()),
    ]);
    p.via = crate::Via(Some(rec.clone()));
    p.sync.started = None;
    let mut host = hosted_via(p);
    let fx = host.update(Event::Tick(NOW));
    assert_eq!(spawned(&fx), 2, "the channel and For you, at once");
    settle(&mut host, fx);
    let methods = rec.methods();
    let count = |m: &str| methods.iter().filter(|x| **x == m).count();
    assert_eq!(count("conversations.history"), 1, "{methods:?}");
    assert_eq!(count("conversations.replies"), 1, "the thread with an unread reply: {methods:?}");
    assert_eq!(count("search.messages"), 3, "{methods:?}");
    assert!(count("users.info") >= 1, "{methods:?}");
    let queries: Vec<String> = rec
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == "search.messages")
        .filter_map(|r| r.params.iter().find(|(k, _)| k == "query").map(|(_, v)| v.clone()))
        .collect();
    assert!(queries[0].starts_with(&format!("<@{ALICE}> after:2026-09-29")), "{queries:?}");
    assert!(queries[1].starts_with("to:me after:"), "{queries:?}");
    assert!(queries[2].starts_with(&format!("<!subteam^{GROUP}> after:")), "{queries:?}");
    let text = shown(&host, 160, 40);
    assert!(text.contains("Carol C"), "the name read: {text}");
    keys(&mut host, &[Key::Char('1'), Key::Down, Key::Down]);
    let text = shown(&host, 160, 40);
    assert!(!text.contains("joined"), "joins are left out: {text}");
    assert!(text.contains("ask mr"), "{text}");
}

#[test]
fn a_failed_read_says_so_in_the_rail_and_the_status_line() {
    let none = || ok(serde_json::json!({"ok": true, "messages": {"matches": [], "paging": {"pages": 1}}}));
    let rec = ByMethod::new(vec![
        ("conversations.info", ok(serde_json::json!({"ok": false, "error": "not_in_channel"}))),
        ("search.messages", none()),
        ("search.messages", none()),
        ("search.messages", none()),
    ]);
    let mut p = sample();
    p.via = crate::Via(Some(rec));
    p.cfg.channels.truncate(1);
    p.sync.started = None;
    let mut host = hosted_via(p);
    let fx = host.update(Event::Tick(NOW));
    settle(&mut host, fx);
    let text = shown(&host, 160, 40);
    assert!(text.contains(":error: #team could not sync: you are not in"), "{text}");
    assert!(text.contains("1 read failed"), "{text}");
    assert!(text.contains(":channel: #team  :error: ✗"), "{text}");
}

#[test]
fn catchup_prints_what_is_new_per_channel_and_as_json() {
    let p = sample();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let history = serde_json::json!({"ok": true, "messages": [
        {"ts": "1791270000.000100", "user": BOB, "text": format!("<!subteam^{GROUP}> :gitlab: <{MR_263}|mr> please"), "reply_count": 2},
        {"ts": "1791260000.000100", "user": CAROL, "text": "morning"}
    ]});
    let rec = crate::client::Recorded::new(vec![ok(serde_json::json!({"ok":true,"team_id":"T000ACME1","user_id":ALICE,"url":"https://acme.slack.com/"})), ok(history.clone())]);
    let c = crate::client::Client::new(rec.clone());
    let text = rt
        .block_on(crate::catchup(&c, p.cfg.clone(), p.cache_snapshot(), Some("team".into()), false, NOW))
        .unwrap();
    assert!(text.starts_with("#team · 2 new in the last 7 days"), "{text}");
    assert!(text.contains("Bob Builder") && text.contains("@review-squad :gitlab: mr please  · 2 replies"), "{text}");
    insta::assert_snapshot!("catchup_text", text);
    let rec = crate::client::Recorded::new(vec![ok(serde_json::json!({"ok":true,"team_id":"T000ACME1","user_id":ALICE,"url":"https://acme.slack.com/"})), ok(history)]);
    let c = crate::client::Client::new(rec);
    let json = rt
        .block_on(crate::catchup(&c, p.cfg.clone(), p.cache_snapshot(), Some("team".into()), true, NOW))
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Oldest first, as the text.
    assert_eq!(v[0]["author"]["name"], "Carol Coder");
    assert_eq!(v[1]["author"]["name"], "Bob Builder");
    assert_eq!(v[1]["updated"], "2026-10-06T07:00:00Z");
    assert!(v[1]["url"].as_str().unwrap().starts_with("https://acme.slack.com/archives/"));
    assert_eq!(v[1]["links"][0], MR_263);
    assert_eq!(v[1]["replies"], 2);
}

#[test]
fn slash_filters_the_list_by_words_and_colon_says_it_goes_nowhere_yet() {
    let mut host = hosted();
    keys(&mut host, &[Key::Char('1'), Key::Down]);
    assert!(shown(&host, 160, 40).contains("Anyone seen ABC-12"), "every thread is there at first");
    press(&mut host, Key::Char('/'));
    assert!(host.plugins[0].capturing());
    chars(&mut host, "lunch");
    let text = shown(&host, 160, 40);
    assert!(text.contains("lunch") && text.contains("Lunch at 12:30"), "{text}");
    assert!(!text.contains("Anyone seen ABC-12"), "only the rows with the words: {text}");
    press(&mut host, Key::Enter);
    assert!(!host.plugins[0].capturing());
    assert!(!shown(&host, 160, 40).contains("Anyone seen ABC-12"), "kept after ⏎");
    // esc with the words open clears them.
    press(&mut host, Key::Char('/'));
    press(&mut host, Key::Esc);
    assert!(shown(&host, 160, 40).contains("Anyone seen ABC-12"));
    press(&mut host, Key::Char(':'));
    assert!(host.message.as_deref().unwrap_or("").contains("not on Slack yet"), "{:?}", host.message);
}

#[test]
fn list_prints_threads_newest_first_filtered_by_words_with_people_as_name_and_handle() {
    let p = sample();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let history = serde_json::json!({"ok": true, "messages": [
        {"ts": "1791260000.000100", "user": CAROL, "text": "morning"},
        {"ts": "1791270000.000100", "user": BOB, "text": "lunch at 12:30?", "reply_count": 1}
    ]});
    let who = |id: &str, name: &str| ok(serde_json::json!({"ok": true, "user": {"id": id, "name": name, "profile": {"real_name": name.to_uppercase()}}}));
    let run = |filter: Option<&str>, json: bool| {
        let rec = crate::client::Recorded::new(vec![ok(serde_json::json!({"ok":true,"team_id":"T000ACME1","user_id":ALICE,"url":"https://acme.slack.com/"})), ok(history.clone()), who(CAROL, "carol"), who(BOB, "bob")]);
        let c = crate::client::Client::new(rec);
        rt.block_on(crate::list(&c, p.cfg.clone(), p.cache_snapshot(), Some("team".into()), filter.map(String::from), json, NOW))
            .unwrap()
    };
    let text = run(None, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("lunch at 12:30?") && lines[0].contains("· 1 reply"), "{text}");
    assert!(lines[1].contains("morning"), "newest first: {text}");
    let only = run(Some("LUNCH bob"), false);
    assert_eq!(only.lines().count(), 1, "{only}");
    let v: serde_json::Value = serde_json::from_str(&run(None, true)).unwrap();
    assert_eq!(v[0]["author"], serde_json::json!({"name": "BOB", "handle": "bob"}));
    assert_eq!(v[0]["updated"], "2026-10-06T07:00:00Z");
    assert!(v[0]["url"].as_str().unwrap().starts_with("https://acme.slack.com/archives/"));
}

// --- with other parts: merge requests and tickets ------------------------------------------------

#[test]
fn merge_request_rows_carry_the_review_ask_and_only_where_it_applies() {
    let host = hosted_with_parts();
    let text = shown(&host, 160, 40);
    assert!(text.contains("api !263 Add a stats endpoint for the dashboard [asked 2d · 2 replies]"), "{text}");
    let other = text.lines().find(|l| l.contains("api !270")).unwrap();
    assert!(!other.contains('['), "no chip: {other}");
}

/// #36: a thread in "Discussed in Slack" names its author in full and the
/// message words carry the link to the message in Slack.
#[test]
fn discussed_in_slack_names_the_author_in_full_and_links_the_message() {
    let host = hosted_with_parts();
    let got = radar_core::Plugin::contribute(host.plugins[2].as_ref(), radar_core::Point::Section, &stand_in::mr(263));
    let Some(radar_core::Contribution::Section { lines, .. }) = got else { panic!("{got:?}") };
    let header = lines.iter().find(|l| l.words().contains("Carol Coder")).expect("the author in full");
    assert!(!header.words().contains("Carol C."), "{header:?}");
    let q = ago(86_400 + 600).replace('.', "");
    let url = format!("https://acme.slack.com/archives/{TEAM}/p{q}");
    let said = lines.iter().find(|l| l.words().contains("Anyone seen ABC-12")).expect("the message");
    assert!(said.0.iter().any(|s| s.link.as_deref() == Some(url.as_str())), "the words link to the message: {said:?}");
}

#[test]
fn the_merge_request_page_has_a_discussed_in_slack_section() {
    let mut host = hosted_with_parts();
    press(&mut host, Key::Enter);
    let text = shown(&host, 160, 40);
    assert!(text.contains("== Discussed in Slack"), "{text}");
    assert!(text.contains("review asked in #team 2d ago · 2 replies · 👀 1"), "{text}");
    assert!(!text.contains("#team · Bob Builder · 2d ago"), "the ask's thread is said once: {text}");
    assert!(text.contains("#team · Carol Coder · 1d ago · 1 reply"), "linked from a reply: {text}");
    assert!(text.contains("Anyone seen ABC-12 failing on main?"), "what was said: {text}");
    // No address spelled out: the channel's name is the link.
    assert!(!text.contains("https://acme.slack.com"), "{text}");
    let q = ago(86_400 + 600).replace('.', "");
    let got = radar_core::Plugin::contribute(host.plugins[2].as_ref(), radar_core::Point::Section, &stand_in::mr(263));
    let Some(radar_core::Contribution::Section { lines, .. }) = got else { panic!("{got:?}") };
    let url = format!("https://acme.slack.com/archives/{TEAM}/p{q}");
    assert!(lines.iter().flat_map(|l| &l.0).any(|s| s.link.as_deref() == Some(url.as_str())), "each thread's link: {lines:?}");
    insta::assert_snapshot!("mr_page_section_160x40", text);
}

/// The merge request's Slack tab: a row per thread whose subject carries its
/// permalink, so the owner's open key opens it and ⏎ shows its page.
#[test]
fn a_merge_request_has_a_slack_tab_of_its_threads_each_with_its_link() {
    let host = hosted_with_parts();
    let got = radar_core::Plugin::contribute(host.plugins[2].as_ref(), radar_core::Point::Tab, &stand_in::mr(263));
    let Some(radar_core::Contribution::Tab { title, rows }) = got else { panic!("{got:?}") };
    assert_eq!(title, "Slack");
    let ask = ago(2 * 86_400 + 3600).replace('.', "");
    let q = ago(86_400 + 600).replace('.', "");
    let urls: Vec<String> = rows.iter().filter_map(|r| r.subject.as_ref()?.url.clone()).collect();
    assert_eq!(
        urls,
        [
            format!("https://acme.slack.com/archives/{TEAM}/p{ask}"),
            format!("https://acme.slack.com/archives/{TEAM}/p{q}"),
        ],
        "the review ask first"
    );
    assert!(rows[0].line.to_string().starts_with("review asked in #team 2d ago · 2 replies"), "{}", rows[0].line);
}

#[test]
fn discussions_group_by_what_the_parts_here_recognise() {
    let mut host = hosted_with_parts();
    press(&mut host, Key::Char('S'));
    keys(&mut host, &[Key::Char('1'), Key::Down]);
    let text = shown(&host, 200, 40);
    assert!(text.contains("api !263"), "{text}");
    assert!(text.contains("ABC-12"), "{text}");
    assert!(text.contains("BY WHAT THEY ARE ABOUT"), "{text}");
    insta::assert_snapshot!("discussions_with_parts_200x40", text);
    // ⏎ on the first group shows the merge request in its owner's page.
    keys(&mut host, &[Key::Char('2'), Key::Char('g'), Key::Enter]);
    let text = shown(&host, 160, 40);
    assert!(text.contains("merge request page api !263"), "{text}");
    press(&mut host, Key::Esc);
    assert_eq!(host.screen().unwrap().1.name, "slack", "esc comes back");
}

#[test]
fn the_ticket_page_has_a_slack_tab_whose_rows_open_the_thread() {
    let mut host = hosted_with_parts();
    press(&mut host, Key::Char('T'));
    press(&mut host, Key::Enter);
    press(&mut host, Key::Char(']'));
    let text = shown(&host, 160, 40);
    assert!(text.contains("Overview  [Slack]"), "{text}");
    assert!(text.contains("#team · Carol Coder · 1d ago · 1 reply  Anyone seen ABC-12 failing on main?"), "{text}");
    insta::assert_snapshot!("ticket_slack_tab_160x40", text);
    press(&mut host, Key::Enter);
    assert_eq!(host.screen().unwrap().1.name, "slack");
    let text = shown(&host, 160, 40);
    assert!(text.contains("Slack / #team / Carol Coder: Anyone seen ABC-12"), "{text}");
    assert!(text.contains("can you look?"), "{text}");
    press(&mut host, Key::Esc);
    assert_eq!(host.screen().unwrap().1.name, "tickets", "back to the ticket");
}

#[test]
fn a_slack_link_in_another_part_opens_its_thread() {
    let mut host = hosted_with_parts();
    let ts = ago(86_400 + 600);
    let link = format!("https://acme.slack.com/archives/{TEAM}/p{}", ts.replace('.', ""));
    let found = host.plugins[2].recognise(&format!("see {link} please"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.id, format!("{TEAM}/{ts}"));
    let fx = host.update(crate::keys::ev(crate::keys::chord(Key::Char('C'), Mods::SHIFT)));
    let _ = fx;
    let _ = settle(&mut host, vec![]);
    // Shown as a bare link from anywhere, it lands on the thread page.
    host.plugins[0] = Box::new(stand_in::Code::default());
    let mut p = sample();
    let shown = radar_core::Plugin::show(&mut p, &found[0].1);
    assert!(matches!(shown, radar_core::Showing::Page { screen: "feed", .. }));
}

mod review_draft {
    use super::*;

    #[test]
    fn snapshots_and_edit_drop_share_one_host() {
        use radar_core::view::{Intent, Node, PaneId};

        // Preserve the six existing golden targets under this narrow scope.
        let mut snapshots = insta::Settings::clone_current();
        snapshots.set_prepend_module_to_snapshot(false);
        let _snapshots = snapshots.bind_to_scope();

        fn spawns(effect: &HostEffect) -> bool {
            match effect {
                HostEffect::Spawn { .. } => true,
                HostEffect::Tracked { effect, .. } => spawns(effect),
                _ => false,
            }
        }
        fn dispatch(host: &mut Host, event: Event) {
            let effects = host.update(event);
            assert!(
                !effects.iter().any(spawns),
                "review-ask drafting, editing and dropping must not spawn"
            );
        }
        fn key(host: &mut Host, code: Key) {
            dispatch(host, crate::keys::ev(crate::keys::chord(code, Mods::NONE)));
        }
        fn select_mr(host: &mut Host, iid: &str) {
            assert_eq!(host.screen().unwrap().1.name, "code");
            let view = host.screen_view_in(0, "code", 160, 40).expect("Code view");
            let Node::List { rows, .. } = &view.pane(PaneId::Middle).expect("merge requests").body
            else {
                panic!("merge request rows");
            };
            let target = rows
                .iter()
                .find(|row| row.target.as_str() == iid)
                .expect("emitted merge request target")
                .target
                .clone();
            dispatch(host, Event::Intent(Intent::Select { target }));
        }
        fn drop_draft(host: &mut Host) {
            key(host, Key::Char('n'));
            assert_eq!(
                host.menu.as_ref().expect("drop confirmation").menu.title,
                "drop this review ask?"
            );
            key(host, Key::Char('y'));
            assert!(host.menu.is_none());
            assert!(!host.plugins[2].capturing());
            assert_eq!(host.message.as_deref(), Some("review ask draft dropped"));
        }

        // Configuration starts with an empty placeholder. Only the installed
        // sample loads a demo world; its recorded transport never reaches Slack.
        let rec = crate::client::Recorded::new(vec![]);
        let mut host = Host::new(
            vec![
                Box::new(stand_in::Code::default()),
                Box::new(stand_in::Tracker::default()),
                Box::new(SlackPlugin::default()),
            ],
            NOW,
        )
        .unwrap();
        host.load_config(Some("")).unwrap();
        let mut p = sample();
        forget_the_review_ask(&mut p);
        p.via = crate::Via(Some(rec.clone()));
        host.plugins[2] = Box::new(p);

        key(&mut host, Key::Char('x'));
        let labels = menu_labels(&host);
        assert!(
            labels.iter().any(|l| l == " ask someone"),
            "under what it is for: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|l| l.ends_with("ask for review in Slack…")),
            "{labels:?}"
        );
        insta::assert_snapshot!("radar_slack__tests__code_keys_with_slack", menu_shown(&host));
        key(&mut host, Key::Esc);
        // The contributed action opens the real Slack draft page.
        key(&mut host, Key::Char('s'));
        assert_eq!(host.screen().unwrap().1.name, "slack");
        let text = shown(&host, 160, 40);
        assert!(text.contains("Ask for review in #team"), "{text}");
        assert!(
            text.contains(&format!("@review-squad :gitlab: mr <{MR_263}> Add a stats endpoint for the dashboard")),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "<!subteam^{GROUP}> :gitlab: <{MR_263}|mr> Add a stats endpoint for the dashboard"
            )),
            "the text sent: {text}"
        );
        insta::assert_snapshot!("radar_slack__tests__draft_one", text);
        key(&mut host, Key::Char('x'));
        insta::assert_snapshot!("radar_slack__tests__draft_keys", menu_shown(&host));
        key(&mut host, Key::Esc);
        // Esc keeps the first item; the next contributed action adds another.
        key(&mut host, Key::Esc);
        assert_eq!(host.screen().unwrap().1.name, "code");
        select_mr(&mut host, "270");
        key(&mut host, Key::Char('x'));
        assert!(
            menu_labels(&host)
                .iter()
                .any(|l| l.ends_with("add to the review ask (1 in it)…")),
            "{:?}",
            menu_labels(&host)
        );
        key(&mut host, Key::Esc);
        key(&mut host, Key::Char('s'));
        let text = shown(&host, 160, 40);
        assert!(
            text.contains(&format!("• api !263 <{MR_263}> Add a stats endpoint for the dashboard")),
            "{text}"
        );
        assert!(
            text.contains("• api !270 <https://git.example/acme/team-a/api/-/merge_requests/270> Tidy the README"),
            "{text}"
        );
        insta::assert_snapshot!("radar_slack__tests__draft_two_160x40", text);

        // Choose the emitted test channel through the actual menu and Enter.
        key(&mut host, Key::Char('c'));
        let at = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|i| i.value == "try")
            .unwrap();
        host.menu.as_mut().unwrap().row = at;
        key(&mut host, Key::Enter);
        let text = shown(&host, 160, 40);
        assert!(text.contains("Ask for review in #try"), "{text}");
        assert!(!text.contains("subteam"), "{text}");
        assert!(
            text.contains("not the review channel: the user group is not mentioned here"),
            "{text}"
        );
        insta::assert_snapshot!("radar_slack__tests__draft_test_channel_160x40", text);

        // Finish the visual journey, then recreate the original single-item
        // edit/drop case through real navigation rather than private state.
        drop_draft(&mut host);
        key(&mut host, Key::Char('C'));
        select_mr(&mut host, "263");
        key(&mut host, Key::Char('s'));
        key(&mut host, Key::Char('e'));
        assert!(host.plugins[2].capturing());
        for _ in 0..60 {
            key(&mut host, Key::Backspace);
        }
        for c in "stats for the dashboard <3 & more".chars() {
            key(&mut host, Key::Char(c));
        }
        key(&mut host, Key::Enter);
        assert!(!host.plugins[2].capturing());
        let text = shown(&host, 160, 40);
        assert!(
            text.contains(&format!(
                "<{MR_263}|mr> stats for the dashboard &lt;3 &amp; more"
            )),
            "{text}"
        );
        drop_draft(&mut host);
        assert!(
            rec.requests().is_empty(),
            "no provider request in this draft contract"
        );
    }
}

mod review_post {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn success_and_refusal_share_one_host_and_real_tasks() {
        use radar_core::scenario::Scenario;
        use radar_core::view::{Intent, Node, PaneId};

        const BUDGET: usize = 256;
        let mut snapshots = insta::Settings::clone_current();
        snapshots.set_prepend_module_to_snapshot(false);
        let _snapshots = snapshots.bind_to_scope();

        fn dispatch_without_task(s: &mut Scenario, event: Event) {
            let before = s.costs().spawned_completion_tasks;
            s.dispatch(event).unwrap();
            assert_eq!(
                s.costs().spawned_completion_tasks,
                before,
                "navigation, preview and decline must not spawn"
            );
        }
        fn key(s: &mut Scenario, code: Key) {
            dispatch_without_task(s, crate::keys::ev(crate::keys::chord(code, Mods::NONE)));
        }
        fn accept(s: &mut Scenario) {
            let before = s.costs().spawned_completion_tasks;
            s.dispatch(crate::keys::ev(crate::keys::chord(
                Key::Char('y'),
                Mods::NONE,
            )))
            .unwrap();
            assert_eq!(
                s.costs().spawned_completion_tasks,
                before + 1,
                "one real task for one accepted review ask"
            );
        }
        fn select_mr(s: &mut Scenario, iid: &str) {
            let target = s.inspect(|host| {
                assert_eq!(host.screen().unwrap().1.name, "code");
                let view = host.screen_view_in(0, "code", 160, 40).expect("Code view");
                let Node::List { rows, .. } =
                    &view.pane(PaneId::Middle).expect("merge requests").body
                else {
                    panic!("merge request rows");
                };
                rows.iter()
                    .find(|row| row.target.as_str() == iid)
                    .expect("emitted merge request target")
                    .target
                    .clone()
            });
            dispatch_without_task(s, Event::Intent(Intent::Select { target }));
        }

        // Literal wire contracts are independent of the production request builder.
        // Recorded supplies replies in order; exact matching is asserted afterward.
        let expected = [
            crate::client::Request {
                method: "chat.postMessage",
                params: vec![],
                write: true,
                body: Some(serde_json::json!({
                    "channel": TEAM,
                    "text": format!("<!subteam^{GROUP}> :gitlab: <https://git.example/acme/team-a/api/-/merge_requests/270|mr> Tidy the README"),
                    "unfurl_links": false,
                    "unfurl_media": false,
                })),
            },
            crate::client::Request {
                method: "chat.postMessage",
                params: vec![],
                write: true,
                body: Some(serde_json::json!({
                    "channel": TEAM,
                    "text": format!("<!subteam^{GROUP}> :gitlab: <{MR_263}|mr> Add a stats endpoint for the dashboard"),
                    "unfurl_links": false,
                    "unfurl_media": false,
                })),
            },
        ];
        let rec = crate::client::Recorded::new(vec![
            ok(serde_json::json!({"ok": true, "channel": TEAM, "ts": "1791271000.000500"})),
            Ok(crate::client::Reply {
                status: 429,
                retry_after: Some(30),
                ..Default::default()
            }),
        ]);
        let mut host = Host::new(
            vec![
                Box::new(stand_in::Code::default()),
                Box::new(stand_in::Tracker::default()),
                Box::new(SlackPlugin::default()),
            ],
            NOW,
        )
        .unwrap();
        host.load_config(Some("")).unwrap();
        let mut p = sample();
        forget_the_review_ask(&mut p);
        p.via = crate::Via(Some(rec.clone()));
        host.plugins[2] = Box::new(p);
        let mut s = Scenario::new(host);

        // The first request has no previous review ask. Keep the exact real
        // confirmation rows, including the payload's 72-cell line break.
        select_mr(&mut s, "270");
        key(&mut s, Key::Char('s'));
        key(&mut s, Key::Char('p'));
        assert_eq!(
            s.host().menu.as_ref().unwrap().menu.title,
            "post this in #team?"
        );
        assert_eq!(
            menu_labels(s.host()),
            [
                format!(" <!subteam^{GROUP}> :gitlab: <https://git.example/acme/team-a/api/-/me"),
                " rge_requests/270|mr> Tidy the README".into(),
                "y yes".into(),
                "e edit".into(),
                "n no".into(),
            ],
            "exactly the text that is posted"
        );
        assert!(rec.requests().is_empty(), "preview cannot send");
        key(&mut s, Key::Char('n'));
        assert_eq!(s.costs().spawned_completion_tasks, 0);
        assert!(rec.requests().is_empty());
        assert_eq!(
            s.host().message.as_deref(),
            Some("nothing sent · p has your text")
        );
        key(&mut s, Key::Char('p'));
        assert!(rec.requests().is_empty(), "reopening cannot send");
        accept(&mut s);
        s.settle(BUDGET).await.unwrap();
        assert_eq!(s.costs().spawned_completion_tasks, 1);
        assert_eq!(
            rec.requests().as_slice(),
            &expected[..1],
            "sent exactly once"
        );
        assert_eq!(rec.replies.lock().unwrap().len(), 1);
        assert_eq!(
            s.host().message.as_deref(),
            Some("✓ review asked · its thread is followed on the merge request")
        );
        key(&mut s, Key::Char('C'));
        let text = s.inspect(|host| shown(host, 160, 40));
        assert!(
            text.contains("api !270 Tidy the README [asked 0s · no reply]"),
            "{text}"
        );

        // Success closed the first draft. Start the original refusal subject
        // through the contributed action, preserving the existing golden state.
        select_mr(&mut s, "263");
        key(&mut s, Key::Char('s'));
        key(&mut s, Key::Char('p'));
        assert_eq!(
            rec.requests().as_slice(),
            &expected[..1],
            "second preview cannot send"
        );
        accept(&mut s);
        s.settle(BUDGET).await.unwrap();
        assert_eq!(s.costs().spawned_completion_tasks, 2);
        assert_eq!(
            rec.requests(),
            expected,
            "one request per accepted ask, never retried"
        );
        let refusal = s
            .host()
            .message
            .as_deref()
            .expect("the refused ask reports why")
            .to_owned();
        assert!(refusal.contains("✗ Slack did not post the review ask in #team: Slack is busy (rate limited): it was not sent"));
        let text = s.inspect(|host| shown(host, 160, 40));
        assert!(
            text.contains("Slack did not take it: Slack is busy (rate limited): it was not sent"),
            "{text}"
        );
        insta::assert_snapshot!("radar_slack__tests__draft_refused_160x40", text);
        key(&mut s, Key::Char('~'));
        let history = s.inspect(|host| {
            host.chrome_view(160, 40)
                .notice_history
                .expect("the refusal remains accessible")
        });
        assert_eq!(history.selected, 0);
        let detail = history.detail.expect("the full refusal is selected");
        assert_eq!(detail.text, refusal);
        assert_eq!(detail.tone, radar_core::view::chrome::NoticeTone::Error);
        assert_eq!(
            rec.requests(),
            expected,
            "reading history cannot retry a refused ask"
        );
        key(&mut s, Key::Esc);

        s.settle(BUDGET).await.unwrap();
        assert!(s.is_idle());
        assert_eq!(s.costs().spawned_completion_tasks, 2);
        assert_eq!(
            rec.requests(),
            expected,
            "settling does not retry the refused ask"
        );
        assert!(
            rec.replies.lock().unwrap().is_empty(),
            "both responses were consumed"
        );
        assert!(s.desktop_effects().is_empty());
        s.finish_journal().unwrap();
    }
}

mod reply_confirmation {
    use super::*;

    #[test]
    fn snapshots_and_edit_decline_recovery_share_one_host() {
        use radar_core::view::{Editor, Intent, Node, PaneId, TreeRow};

        // Keep checking the original files despite the smaller classification scope.
        let mut snapshots = insta::Settings::clone_current();
        snapshots.set_prepend_module_to_snapshot(false);
        let _snapshots = snapshots.bind_to_scope();

        // Only the two golden checkpoints render. Every interaction still reaches
        // the real host; even a tracked spawn before a send is a contract failure.
        fn spawns(effect: &HostEffect) -> bool {
            match effect {
                HostEffect::Spawn { .. } => true,
                HostEffect::Tracked { effect, .. } => spawns(effect),
                _ => false,
            }
        }
        fn dispatch(host: &mut Host, event: Event) {
            let effects = host.update(event);
            assert!(
                !effects.iter().any(spawns),
                "editing or retaining a reply must not spawn"
            );
        }
        fn key(host: &mut Host, code: Key) {
            dispatch(host, crate::keys::ev(crate::keys::chord(code, Mods::NONE)));
        }
        fn preview(host: &mut Host) {
            dispatch(
                host,
                crate::keys::ev(crate::keys::chord(Key::Char('s'), Mods::CONTROL)),
            );
        }
        fn composer_text(host: &Host) -> String {
            let view = host.screen_view_in(0, "feed", 160, 40).expect("Slack feed");
            let editor =
                Editor::of(&view.overlay.expect("reply overlay").body).expect("reply editor");
            editor
                .lines
                .into_iter()
                .chain(std::iter::once(editor.line))
                .collect::<Vec<_>>()
                .join("\n")
        }

        let (p, rec) = recorded(vec![]);
        let mut host = hosted_via(p);
        // Select actual emitted targets, rather than drawing once per arrow key.
        key(&mut host, Key::Char('1'));
        let view = host.screen_view_in(0, "feed", 160, 40).expect("Slack feed");
        let Node::List { rows, .. } = &view.pane(PaneId::Nav).expect("Slack navigation").body
        else {
            panic!("Slack navigation rows");
        };
        let channel = rows
            .iter()
            .find(|row| row.text.to_string().contains("#team"))
            .expect("team channel")
            .target
            .clone();
        dispatch(&mut host, Event::Intent(Intent::Select { target: channel }));
        key(&mut host, Key::Char('2'));
        let view = host.screen_view_in(0, "feed", 160, 40).expect("team feed");
        let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("channel messages").body else {
            panic!("channel message tree");
        };
        let root = ago(2 * 86_400 + 3600);
        let path = tree
            .visible()
            .into_iter()
            .find(|(path, _)| path.last().is_some_and(|id| id.contains(&root)))
            .expect("review ask root")
            .0;
        dispatch(
            &mut host,
            Event::Intent(Intent::Select {
                target: TreeRow::target_of(&path),
            }),
        );
        key(&mut host, Key::Char('c'));
        assert!(host.plugins[0].capturing());
        // The kit's composer: ⏎ a new line, ctrl-s review, esc stop.
        assert_eq!(
            host.plugins[0]
                .hints("slack", radar_core::Pane::Middle)
                .iter()
                .map(|h| h.keys)
                .collect::<Vec<_>>(),
            ["⏎", "ctrl-s", "esc", "ctrl-r"]
        );
        for c in "looking now, <@U00000BOB1>".chars() {
            key(&mut host, Key::Char(c));
        }
        key(&mut host, Key::Enter);
        for c in "second line".chars() {
            key(&mut host, Key::Char(c));
        }
        let body = "looking now, <@U00000BOB1>\nsecond line";
        assert_eq!(composer_text(&host), body);
        assert!(rec.requests().is_empty(), "typing cannot send");
        let text = shown(&host, 160, 40);
        assert!(
            text.contains("✎ reply in #team, in Bob Builder's thread"),
            "{text}"
        );
        assert!(text.contains("looking now, <@U00000BOB1>"), "{text}");
        insta::assert_snapshot!("radar_slack__tests__reply_typing_160x40", text);
        preview(&mut host);
        let menu = &host.menu.as_ref().unwrap().menu;
        assert_eq!(menu.title, "post this in #team, in Bob Builder's thread?");
        assert_eq!(
            menu.items
                .iter()
                .map(|i| format!("{} {} · {}", i.keys, i.label, i.hint))
                .collect::<Vec<_>>(),
            [
                " looking now, <@U00000BOB1> · ",
                " second line · ",
                "y yes · yes, post it",
                "e edit · keep writing",
                "n no · no, drop it",
            ]
        );
        assert!(rec.requests().is_empty(), "preview cannot send");
        insta::assert_snapshot!("radar_slack__tests__reply_confirm", menu_shown(&host));
        // e: back to the text, as it was.
        key(&mut host, Key::Char('e'));
        assert!(host.plugins[0].capturing() && rec.requests().is_empty());
        assert_eq!(composer_text(&host), body, "edit preserves every line");
        preview(&mut host);
        // n: nothing sent, the composer closes, c has the text back.
        key(&mut host, Key::Char('n'));
        assert_eq!(
            host.message.as_deref(),
            Some("nothing sent · c has your text")
        );
        assert!(!host.plugins[0].capturing());
        assert!(rec.requests().is_empty(), "declining cannot send");
        key(&mut host, Key::Char('c'));
        key(&mut host, Key::Enter); // choose the retained occurrence
        let details = &host.menu.as_ref().expect("retained reply details").menu;
        for line in body.lines() {
            assert!(
                details.items.iter().any(|item| item.label.contains(line)),
                "retained line: {line}"
            );
        }
        key(&mut host, Key::Char('e')); // explicitly recover; no send
        assert!(host.plugins[0].capturing());
        assert_eq!(composer_text(&host), body, "recovery preserves every line");
        // esc with text asks first.
        key(&mut host, Key::Esc);
        assert_eq!(
            host.menu.as_ref().unwrap().menu.title,
            "drop this reply draft?"
        );
        assert_eq!(composer_text(&host), body, "asking to drop keeps the draft");
        key(&mut host, Key::Char('n'));
        assert!(host.plugins[0].capturing());
        assert_eq!(
            composer_text(&host),
            body,
            "declining to drop keeps the draft"
        );
        key(&mut host, Key::Esc);
        key(&mut host, Key::Char('y')); // this yes drops the local draft only
        assert!(!host.plugins[0].capturing());
        assert!(
            host.screen_view_in(0, "feed", 160, 40)
                .unwrap()
                .overlay
                .is_none()
        );
        assert!(
            rec.requests().is_empty(),
            "no provider request in this composer contract"
        );
    }
}

#[test]
fn each_mode_sends_only_the_writes_it_allows_and_its_question_says_so() {
    use radar_core::writes::Mode;
    // Every Slack write adds: write and edit send it, read only does not.
    for mode in Mode::ALL {
        let (p, rec) = recorded(vec![
            ok(serde_json::json!({"ok": true, "channel": TEAM, "ts": "1791271000.000900"})),
            ok(serde_json::json!({"ok": true, "messages": []})),
            ok(serde_json::json!({"ok": true})),
            ok(serde_json::json!({"ok": true, "messages": []})),
        ]);
        let mut host = hosted_via(p);
        host.set_env_mode(Some((mode, "RADAR_MODE".into())));
        on_team(&mut host, 4);
        // The x menu marks the writes the mode refuses.
        press(&mut host, Key::Char('x'));
        let reply_row = host.menu.as_ref().unwrap().menu.items.iter().find(|i| i.label.contains("comment in its thread")).unwrap().clone();
        assert!(!reply_row.label.contains("radar is read only"), "the reason is not in the label: {reply_row:?}");
        assert_eq!(reply_row.hint.contains("radar is read only"), mode == Mode::ReadOnly, "{mode:?}: {reply_row:?}");
        press(&mut host, Key::Esc);
        press(&mut host, Key::Char('c'));
        chars(&mut host, "hi");
        host.update(crate::keys::ev(crate::keys::chord(Key::Char('s'), Mods::CONTROL)));
        let shown = menu_labels(&host);
        assert_eq!(
            shown[1] == " radar is read only (set by RADAR_MODE): nothing will be sent",
            mode == Mode::ReadOnly,
            "{mode:?}: {shown:?}"
        );
        let fx = press(&mut host, Key::Char('y'));
        settle(&mut host, fx);
        if mode == Mode::ReadOnly {
            assert!(rec.requests().is_empty(), "nothing leaves");
            assert_eq!(
                host.message.as_deref(),
                Some("✗ reply not sent · c has your text · ctrl-r lists kept replies")
            );
        } else {
            assert_eq!(rec.requests()[0].method, "chat.postMessage", "{mode:?}");
        }
        // A reaction too.
        press(&mut host, Key::Char('e'));
        press(&mut host, Key::Char('e'));
        assert_eq!(
            menu_labels(&host).iter().any(|l| l.contains("radar is read only")),
            mode == Mode::ReadOnly,
            "{:?}",
            menu_labels(&host)
        );
        let fx = press(&mut host, Key::Char('y'));
        assert_eq!(spawned(&fx), usize::from(mode != Mode::ReadOnly), "{mode:?}: react");
        if mode == Mode::ReadOnly {
            assert!(host.message.as_deref().is_some_and(|m| m.starts_with("✗ Slack did not react :eyes: in #team: not sent")), "{:?}", host.message);
        }
    }
}

#[test]
fn a_reply_names_whose_thread_it_is_when_only_its_page_holds_it() {
    let (mut p, _) = recorded(vec![]);
    // A thread fetched for its page only: not in the channels held.
    p.threads.insert(
        crate::thread_key(TEAM, "1791200000.000100"),
        crate::Fetched {
            value: Some(vec![Message {
                ts: "1791200000.000100".into(),
                user: Some(BOB.into()),
                text: "old question".into(),
                ..Default::default()
            }]),
            ..Default::default()
        },
    );
    assert!(p.data.thread(TEAM, "1791200000.000100").is_none(), "not held in the channel");
    assert_eq!(p.reply_place(TEAM, "1791200000.000100"), "#team, in Bob Builder's thread");
    assert_eq!(p.reply_place(TEAM, "1791200000.999999"), "#team, in the thread", "never “the's”");
    // A message found for you that starts the thread names it too.
    p.data.for_you.push(crate::client::Found {
        channel_id: TEAM.into(),
        message: Message { ts: "1791200000.999999".into(), user: Some(CAROL.into()), ..Default::default() },
        ..Default::default()
    });
    assert_eq!(p.reply_place(TEAM, "1791200000.999999"), "#team, in Carol Coder's thread");
}

#[test]
fn a_reaction_is_picked_asked_and_sent_once() {
    let rec = crate::client::Recorded::new(vec![
        ok(serde_json::json!({"ok": true})),
        ok(serde_json::json!({"ok": true, "messages": []})),
    ]);
    let (mut p, _) = recorded(vec![]);
    p.via = crate::Via(Some(rec.clone()));
    let mut host = hosted_via(p);
    on_team(&mut host, 4);
    press(&mut host, Key::Char('e'));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "react to Bob Builder's message");
    insta::assert_snapshot!("react_menu", menu_shown(&host));
    press(&mut host, Key::Char('e'));
    let menu = &host.menu.as_ref().unwrap().menu;
    assert_eq!(menu.title, "react :eyes: to Bob Builder's message?");
    assert_eq!(menu_labels(&host), [" adds :eyes: to that message in #team", "y yes", "n no"]);
    assert_eq!(menu.items[1].hint, "yes, add it");
    let fx = press(&mut host, Key::Char('n'));
    assert_eq!(spawned(&fx), 0);
    press(&mut host, Key::Char('e'));
    press(&mut host, Key::Char('e'));
    let fx = press(&mut host, Key::Char('y'));
    settle(&mut host, fx);
    let reqs = rec.requests();
    assert_eq!(reqs[0].method, "reactions.add");
    assert_eq!(
        reqs[0].body.clone().unwrap(),
        serde_json::json!({"channel": TEAM, "timestamp": ago(2 * 86_400 + 3600), "name": "eyes"})
    );
}

#[test]
fn not_set_up_it_adds_nothing_to_other_parts() {
    let mut host = hosted_with_parts();
    host.plugins[2] = Box::new(SlackPlugin::new());
    let text = shown(&host, 160, 40);
    assert!(!text.contains("asked"), "{text}");
    press(&mut host, Key::Char('x'));
    assert!(!menu_labels(&host).iter().any(|l| l.contains("Slack")), "{:?}", menu_labels(&host));
}

// --- every row of every menu -------------------------------------------------------------------

/// Every place's `x` rows and click targets on a Slack plugin made by
/// `make` (radar_core::check, on the plugin alone): none is a gap; how many
/// were tried.
pub(crate) fn every_row(make: fn() -> SlackPlugin, places: &[radar_core::check::Place]) -> usize {
    let make = move || Box::new(make()) as Box<dyn radar_core::Plugin>;
    let mut check = radar_core::check::Check::new(&make);
    let gaps = check.places(places);
    assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    check.checked
}

/// The places on the Slack screen: the nav rows `navs`, `downs` rows of
/// the list under each with its detail pane and its page's two tabs, and with
/// `sections` the first rows of each settings section.
pub(crate) fn slack_places_of(navs: std::ops::Range<usize>, downs: usize, sections: bool) -> Vec<radar_core::check::Place> {
    use radar_core::Pane;
    use radar_core::check::Place;
    let mut places = Vec::new();
    for nav in navs {
        let base = Place::screen("feed", Pane::Nav).down(nav);
        places.push(base.clone());
        for down in 0..downs {
            let list = base.clone().focus(Pane::Middle).down(down);
            places.push(list.clone());
            places.push(list.clone().focus(Pane::Detail));
            places.push(list.clone().enter());
            places.push(list.enter().tab(1));
        }
    }
    for s in radar_core::Plugin::sections(&sample()).into_iter().filter(|_| sections) {
        for row in 0..14 {
            places.push(Place::section(s.id).down(row));
        }
    }
    places
}

#[test]
fn every_row_and_target_is_taken() {
    let checked = every_row(sample, &slack_places_of(0..4, 5, true));
    assert!(checked > 300, "only {checked} checked");
}

/// What Slack adds to a merge request's `x` menu (ask for a review, its
/// thread) each returns effects; and the draft page it opens has rows
/// that are taken.
#[test]
fn every_action_slack_adds_acts_and_the_draft_page_rows_are_taken() {
    let make = || Box::new(sample()) as Box<dyn radar_core::Plugin>;
    let mut check = radar_core::check::Check::new(&make);
    let n = check.actions(&stand_in::mr(263)) + check.actions(&stand_in::mr(270));
    assert!(n >= 2, "{n} actions");
    assert!(check.gaps.is_empty(), "{:?}", check.gaps);
    // The draft: asked from !263.
    fn drafted() -> SlackPlugin {
        let mut p = sample();
        let _ = radar_core::Plugin::act(&mut p, &stand_in::mr(263), "ask");
        p
    }
    use radar_core::Pane;
    use radar_core::check::Place;
    let places = [Place::screen("feed", Pane::Middle), Place::screen("feed", Pane::Middle).down(2)];
    assert!(every_row(drafted, &places) > 3);
}

/// Open a settings field's input from its row, empty it and type `w`: the
/// input is on that row, and only there, with `w` typed into it (a stray
/// first letter would look typed but could not be deleted, and ⏎ would
/// drop it). `row` picks the line by its label.
/// The settings input open: the words of the row it is on, and how many
/// cells into that row it starts (`Editing.at`, `TreeEditing.at`).
fn open_input(host: &Host) -> Option<(String, u16)> {
    use radar_core::view::{Node, TreeRow};
    fn find<'a>(rows: &'a [TreeRow], path: &[String]) -> Option<&'a TreeRow> {
        let (first, rest) = path.split_first()?;
        let row = rows.iter().find(|r| &r.id == first)?;
        if rest.is_empty() { Some(row) } else { find(&row.children, rest) }
    }
    let section = radar_core::headless::section_of(host, 160, 40)?;
    match &section.body {
        Node::List { rows, editing: Some(e), .. } => Some((rows.get(e.row)?.text.words(), e.at)),
        Node::Tree(tree) => {
            let e = tree.editing.as_ref()?;
            // A tree's name starts at the pane's edge; a list row a cell in.
            Some((find(&tree.rows, &e.path)?.name.words(), e.at + 1))
        }
        _ => None,
    }
}

/// `input_opens_on_its_row`, and the input starts on the cells `value` was
/// drawn in: `at` is a cell plus the width of the row's words before it.
/// That Ink puts the field `at` cells into the row, the caret on its
/// character, is ink/test/nodes/list-editing-at.test.tsx.
fn input_sits_on_its_value(host: &mut Host, name: &str, downs: usize, open: &[Key], row: &str, value: &str) {
    input_opens_on_its_row(host, name, downs, open, row);
    for code in open {
        host.update(crate::keys::ev(crate::keys::chord(*code, Mods::NONE)));
    }
    let (words, at) = open_input(host).unwrap_or_else(|| panic!("{name}: no input open on {row:?}"));
    let before = words.find(value).unwrap_or_else(|| panic!("{name}: no {value:?} on {words:?}"));
    // A row is drawn a cell in from the pane's edge, its words after that cell.
    assert_eq!(usize::from(at), 1 + radar_core::ui::width(&words[..before]), "{name}, {row:?}: the input starts off its value in {words:?}");
    host.update(crate::keys::ev(crate::keys::chord(Key::Esc, Mods::NONE)));
}

fn input_opens_on_its_row(host: &mut Host, name: &str, downs: usize, open: &[Key], row: &str) {
    let key = |host: &mut Host, code: Key, m: crate::keys::Mods| {
        let _ = shown(host, 160, 40);
        host.update(crate::keys::ev(crate::keys::chord(code, m)))
    };
    host.view = radar_core::host::View::Settings;
    host.section = host.sections.iter().position(|(_, s)| s.name == name).unwrap();
    host.focus = radar_core::Pane::Middle;
    for _ in 0..20 {
        key(host, Key::Up, Mods::NONE);
    }
    for _ in 0..downs {
        key(host, Key::Down, Mods::NONE);
    }
    let before = shown(host, 160, 40);
    assert!(before.lines().any(|l| l.contains(row)), "no row {row:?} in {name}:\n{before}");
    assert!(!before.contains("|]"), "{name}: an input is open before {open:?}:\n{before}");
    for code in open {
        key(host, *code, Mods::NONE);
    }
    assert!(host.plugins[0].capturing(), "{name}: {open:?} on {row:?} opened no input");
    key(host, Key::Char('e'), Mods::CONTROL);
    key(host, Key::Char('u'), Mods::CONTROL);
    // w is text while typing, not the save key.
    let fx = key(host, Key::Char('w'), Mods::NONE);
    assert!(
        !fx.iter().any(|e| matches!(e, radar_core::host::HostEffect::Save(_))),
        "{name}: w saved instead of being typed"
    );
    let after = shown(host, 160, 40);
    let inputs: Vec<&str> = after.lines().filter(|l| l.contains("|]")).collect();
    assert!(
        matches!(inputs[..], [line] if line.contains(row) && line.contains("[w|]")),
        "{name}, {row:?}: the input is not just on that row, holding only w: {inputs:?}\n{after}"
    );
    key(host, Key::Esc, Mods::NONE);
    assert!(!host.plugins[0].capturing());
}

#[test]
fn every_settings_input_starts_where_its_value_was_drawn() {
    use Key::{Char, Enter};
    let mut host = hosted();
    // connection: the api, the token, how often to read, how far back.
    input_sits_on_its_value(&mut host, "connection", 1, &[Char('e')], "api ", "https://slack.com");
    input_sits_on_its_value(&mut host, "connection", 2, &[Char('e')], "token ", "xoxp");
    input_sits_on_its_value(&mut host, "connection", 5, &[Char('e')], "refresh ", "every 2m");
    input_sits_on_its_value(&mut host, "connection", 6, &[Char('e')], "read back ", "7 days");
    // channels: the add row, a channel's alias and purpose.
    input_sits_on_its_value(&mut host, "channels", 0, &[Enter], "+ add a channel", "add a channel");
    input_sits_on_its_value(&mut host, "channels", 2, &[Char('e')], "alias  ", "team");
    input_sits_on_its_value(&mut host, "channels", 3, &[Char('e')], "purpose  ", "where");
    // review: the template, the link text, how long before asking again.
    input_sits_on_its_value(&mut host, "review", 2, &[Char('e')], "template ", "{group}");
    input_sits_on_its_value(&mut host, "review", 3, &[Char('e')], "link text ", "mr");
    input_sits_on_its_value(&mut host, "review", 5, &[Char('e')], "ask again", "after 2");
}

#[test]
fn a_read_says_which_threads_changed_and_the_list_marks_their_rows() {
    use crate::read::Read;
    use radar_core::motion::Cue;
    let mut host = hosted();
    on_team(&mut host, 0);
    host.take_cues();
    let mut messages = sample().data.channels[TEAM].messages.clone();
    messages[1].reply_count = 2;
    messages[1].latest_reply = Some(ago(60));
    let q = crate::join::motion_key(TEAM, &messages[1].ts);
    let read = |messages: Vec<Message>| Event::Msg {
        plugin: 0,
        msg: Box::new(crate::SlackMsg::Read { stamp: Default::default(), read: Box::new(Read::Channel {
            id: TEAM.into(),
            result: Ok(crate::sync_history::ChannelRead {
                info: None,
                list: crate::client::MessagesRead { messages, evidence: Default::default() },
            }),
            names: vec![],
            names_evidence: Default::default(),
            action: crate::sync_history::action("slack.sync.channel", "T000ACME1", TEAM),
        }) }),
    };
    host.update(read(messages.clone()));
    let Some(Cue::Changed(c)) = host.take_cues().pop() else {
        panic!("the read says what changed")
    };
    assert_eq!(c.changed.iter().collect::<Vec<_>>(), [&q], "a new reply in the question's thread");
    assert!(c.added.is_empty());
    host.update(read(messages));
    assert!(host.take_cues().is_empty(), "the same again changes nothing");

    // The row of that thread carries the key its glow is told by.
    use radar_core::view::{Node, PaneId, TreeRow};
    fn glowing<'a>(rows: &'a [TreeRow], key: &str) -> Option<&'a TreeRow> {
        rows.iter().find_map(|r| if r.glow.as_deref() == Some(key) { Some(r) } else { glowing(&r.children, key) })
    }
    let view = host.screen_view(0, crate::SCREEN.id).expect("Slack screen");
    let Node::Tree(tree) = &view.pane(PaneId::Middle).expect("the list").body else { panic!("the list is a tree") };
    let row = glowing(&tree.rows, &q).expect("its row is marked");
    let line = radar_core::headless::text::text(&row.name);
    assert!(line.contains("Anyone seen"), "where it is shown: {line}");
}

// --- icons ---------------------------------------------------------------------------------

#[test]
fn the_screen_and_the_settings_sections_have_icons() {
    use radar_core::Plugin;
    use radar_core::ui::icon::Icon;
    let p = sample();
    assert_eq!(p.icon("slack"), Some(Icon::Chat));
    for s in p.sections() {
        assert!(p.section_icon(s.id).is_some(), "{} has no icon", s.id);
    }
    assert_eq!(p.section_icon("connection"), Some(Icon::Connection));
    assert_eq!(p.section_icon("alerts"), Some(Icon::Alert));
}

/// The host asks every part whether it is set up on every frame, and with
/// no token that asks for the token offers: the process's environment is
/// read once and only names that could hold a Slack token are kept (read
/// and upper-cased whole on each frame, it was nearly half of a frame's
/// work).
#[test]
fn the_environment_is_read_once_for_the_token_offers() {
    let p = SlackPlugin::default();
    let first = p.vars();
    assert!(first.iter().all(|(name, _)| {
        let up = name.to_uppercase();
        up.contains("SLACK") && up.contains("TOKEN")
    }));
    assert!(std::ptr::eq(first, p.vars()), "the second read is the first one kept");
}
