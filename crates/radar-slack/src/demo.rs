//! The sample data the Slack part's tests use, and `radar shot` draws with
//! (`Plugin::demo`): a team channel, a service channel, mentions, a review
//! ask. Neutral names; no live service. Compiled for tests and under the
//! `demo` feature.

use crate::SlackPlugin;
use crate::client::{Found, Me, Message, Reaction, UserGroup};
use crate::config::{Channel, Kind, Settings};
use crate::model::ChannelData;

/// What the host's clock says in tests: 2026-10-06 10:16:40 UTC.
pub(crate) const NOW: i64 = 1_791_271_000;

pub(crate) const TEAM: &str = "C0000TEAM1";
pub(crate) const SVC: &str = "C00000SVC1";
pub(crate) const TRY: &str = "C00000TRY1";
pub(crate) const ALICE: &str = "U0000ALICE";
pub(crate) const BOB: &str = "U00000BOB1";
pub(crate) const CAROL: &str = "U0000CAROL";
pub(crate) const GROUP: &str = "S0REVIEW01";
pub(crate) const MR_263: &str = "https://git.example/acme/team-a/api/-/merge_requests/263";

/// `secs` before now, as a Slack ts.
pub(crate) fn ago(secs: i64) -> String {
    format!("{}.{:06}", NOW - secs, (secs * 7) % 1_000_000)
}

pub(crate) fn msg(ts: &str, user: &str, text: &str) -> Message {
    Message {
        ts: ts.into(),
        user: Some(user.into()),
        text: text.into(),
        ..Message::default()
    }
}

fn channel(alias: &str, id: &str, kind: Kind, color: &str, watched: bool) -> Channel {
    Channel {
        alias: alias.into(),
        id: id.into(),
        name: format!("acme-{alias}"),
        purpose: String::new(),
        kind,
        color: color.into(),
        watched,
        reader: None,
    }
}

pub(crate) fn settings() -> Settings {
    let mut s = Settings {
        token: "xoxp-a-typed-in-token-for-tests".into(),
        ..Settings::default()
    };
    s.channels = vec![
        channel("team", TEAM, Kind::Team, "cyan", true),
        channel("svc", SVC, Kind::Service, "orange", true),
        channel("try", TRY, Kind::Test, "green", false),
    ];
    s.channels[0].purpose = "where the team talks".into();
    s.review.channel = "team".into();
    s.review.group = GROUP.into();
    s.review.handle = "review-squad".into();
    s.review.test_channel = "try".into();
    s
}

/// The team's channel: a review ask (2 replies), a question naming ABC-12
/// with a pipeline link, and today's lunch; a service channel with one
/// message; For you: a mention in the team channel and a direct message.
pub(crate) fn sample() -> SlackPlugin {
    let mut p = SlackPlugin::new();
    p.now = NOW;
    p.cfg = settings();
    p.env = Some(vec![]);
    // No Slack app here, unless a test says.
    p.app_running = Some(false);
    p.sync.started = Some(NOW);
    p.sync.last_ok = Some(NOW - 40);
    p.sync.groups_at = NOW;
    let d = &mut p.data;
    d.me = Some(Me {
        user_id: ALICE.into(),
        user: "alice".into(),
        team: "Acme".into(),
        team_id: "T000ACME1".into(),
        url: "https://acme.slack.com/".into(),
        scopes: [
            "channels:history", "channels:read", "groups:history", "groups:read", "search:read",
            "users:read", "usergroups:read", "chat:write", "files:read",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        tz_offset: Some(0),
    });
    for (id, name) in [(ALICE, "Alice Example"), (BOB, "Bob Builder"), (CAROL, "Carol Coder")] {
        d.users.insert(id.into(), name.into());
    }
    d.groups = vec![UserGroup {
        id: GROUP.into(),
        handle: "review-squad".into(),
        name: "Review squad".into(),
        users: vec![ALICE.into(), BOB.into()],
    }];
    let ask_ts = ago(2 * 86_400 + 3600);
    let q_ts = ago(86_400 + 600);
    let lunch_ts = ago(1800);
    let mut ask = msg(
        &ask_ts,
        BOB,
        &format!("<!subteam^{GROUP}> :gitlab: <{MR_263}|mr> Add a stats endpoint for the dashboard"),
    );
    ask.reply_count = 2;
    ask.reply_users = vec![ALICE.into(), CAROL.into()];
    ask.latest_reply = Some(ago(86_400 * 2));
    ask.reactions = vec![Reaction {
        name: "eyes".into(),
        count: 1,
        users: vec![ALICE.into()],
    }];
    let mut q = msg(
        &q_ts,
        CAROL,
        "Anyone seen *ABC-12* failing on main? <https://git.example/acme/team-a/api/-/pipelines/99|pipeline> and `make check`",
    );
    q.reply_count = 1;
    q.reply_users = vec![ALICE.into()];
    q.latest_reply = Some(ago(86_400));
    let lunch = msg(&lunch_ts, BOB, "Lunch at 12:30? :pizza:\n&gt; bring the good snacks &amp; more");
    let team = ChannelData {
        info: None,
        messages: vec![ask.clone(), q.clone(), lunch],
        replies: [
            (
                ask_ts.clone(),
                vec![
                    msg(&ago(2 * 86_400 + 1800), ALICE, "looking :eyes:"),
                    msg(&ago(2 * 86_400), CAROL, "approved, nice one"),
                ],
            ),
            (q_ts.clone(), vec![msg(&ago(86_400), ALICE, &format!("<@{BOB}> can you look? see {MR_263}"))]),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    d.channels.insert(TEAM.into(), team);
    d.channels.insert(
        SVC.into(),
        ChannelData {
            info: None,
            messages: vec![msg(&ago(5 * 3600), CAROL, "The service is being upgraded at noon")],
            ..Default::default()
        },
    );
    d.for_you = vec![
        Found {
            channel_id: "D0000CAROL".into(),
            channel_name: CAROL.into(),
            is_im: true,
            is_mpim: false,
            permalink: "https://acme.slack.com/archives/D0000CAROL/p1791267400000000".into(),
            message: msg("1791267400.000000", CAROL, &format!("hi <@{ALICE}>, quick question about the stats")),
        },
        Found {
            channel_id: TEAM.into(),
            channel_name: "acme-team".into(),
            is_im: false,
            is_mpim: false,
            permalink: String::new(),
            message: msg(&ago(86_400 + 300), BOB, &format!("<@{ALICE}> could you take this one?")),
        },
    ];
    p.session_owner = p.data.me.as_ref().and_then(crate::cache::AccountOwner::of);
    p
}
