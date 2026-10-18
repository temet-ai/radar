//! Starting work on a ticket from elsewhere: when a workspace says it has
//! started one (bus topic `workspace.started`, payload `{"ticket":"ABC-12",
//! …}`), Jira asks once whether to assign the ticket to you and move it into
//! an in-progress status. The question is a write (it edits), so the mode
//! decides whether `y` sends anything; `n` sends nothing. The assignment and
//! the move are the same requests `i` and `t` make, each sent once, the move
//! after the assignment was taken.
//!
//! `[jira] on_start = "off"` never asks; `[jira] start_status` names the
//! status to move to (otherwise the first move Jira offers into an
//! in-progress status).

use radar_core::plugin::{Effect, Menu, Msg};
use radar_core::journal::{ActionDescriptor, Outcome as JournalOutcome};
use radar_core::ui::icon::Icon;
use serde_json::Value;

#[cfg(test)]
use crate::client::Client;
use crate::config::OnStart;
use crate::model::{Category, Me, Ticket, Transition};
use crate::write::{self, Change};
use crate::{JiraMsg, JiraPlugin, join, write_context};
use crate::write_context::Dispatch;

/// The bus topic Jira listens to; the workspace part emits it.
pub(crate) const TOPIC: &str = "workspace.started";
const MENU: &str = "j-start";

/// The question waiting for its `y`: what it will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Start {
    pub site: String,
    pub key: String,
    pub assign: Option<Change>,
    pub mv: Option<Change>,
}

impl Start {
    /// The question's lines: `assignee: Bob → you`, `status: To Do → In Progress`.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Change::Assign { me, from, .. }) = &self.assign {
            let was = if from.is_empty() { "unassigned" } else { from };
            out.push(format!("assignee: {was} → you ({})", me.name));
        }
        if let Some(Change::Move { to, from, .. }) = &self.mv {
            out.push(format!("status: {from} → {}", to.to));
        }
        out
    }
}

/// The ticket key a `workspace.started` payload names: `"ticket":"ABC-12"`
/// (or an object with a `key`).
pub(crate) fn key_of(payload: &Value) -> Option<String> {
    let t = payload.get("ticket")?;
    let key = t
        .as_str()
        .or_else(|| t.get("key").and_then(Value::as_str))?;
    crate::config::is_ticket_key(key).then(|| key.to_string())
}

/// The first move into an in-progress status, or the one to `wanted`.
pub(crate) fn pick_move<'a>(moves: &'a [Transition], wanted: &str) -> Option<&'a Transition> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return moves.iter().find(|t| t.category == Category::Doing);
    }
    moves
        .iter()
        .find(|t| t.to.eq_ignore_ascii_case(wanted) || t.name.eq_ignore_ascii_case(wanted))
}

/// What starting `t` changes, given who you are and where it can move. A
/// part left out is one already so (or not known): `me` and `moves` are
/// `None` when not known.
pub(crate) fn plan(
    site: &str,
    t: &Ticket,
    me: Option<&Me>,
    moves: Option<&[Transition]>,
    wanted: &str,
) -> Start {
    let assign = me
        .filter(|m| !m.account_id.is_empty() && t.assignee_id != m.account_id)
        .map(|m| Change::Assign {
            site: site.into(),
            key: t.key.clone(),
            me: m.clone(),
            from: t.assignee.clone(),
        });
    let mv = (t.category != Category::Doing)
        .then(|| moves.and_then(|ms| pick_move(ms, wanted)))
        .flatten()
        .map(|to| Change::Move {
            site: site.into(),
            key: t.key.clone(),
            to: to.clone(),
            from: t.status.clone(),
            from_category: t.category,
        });
    Start {
        site: site.into(),
        key: t.key.clone(),
        assign,
        mv,
    }
}

/// `workspace.started`: read what the question needs (who you are, where the
/// ticket can move) in one go, then ask.
pub(crate) fn on_bus(p: &mut JiraPlugin, topic: &str, payload: &Value) -> Vec<Effect> {
    if topic != TOPIC || p.cfg.on_start == OnStart::Off {
        return vec![];
    }
    let Some(key) = key_of(payload) else {
        return vec![];
    };
    let project = key.split('-').next().unwrap_or_default();
    let Some(site) = join::site_for(p, &key) else {
        return vec![];
    };
    let Some(t) = join::held(p, &site, &key) else {
        // Not ours to say anything about unless Jira watches its project.
        if p.cfg.site(&site).is_some_and(|s| s.watches(project)) {
            return vec![Effect::Notify(format!(
                "{key} is not fetched from Jira yet: not asked to start it"
            ))];
        }
        return vec![];
    };
    let me = write::me(p, &site).cloned();
    let mine = me.as_ref().is_some_and(|m| m.account_id == t.assignee_id);
    let need_me = me.is_none();
    let need_moves = t.category != Category::Doing;
    if !need_me && mine && !need_moves {
        return vec![];
    }
    let (Some(s), true) = (
        p.cfg.site(&site).cloned(),
        p.asking.insert(format!("start\0{site}\0{key}")),
    ) else {
        return vec![];
    };
    let k = key.clone();
    let provider = p.provider.clone();
    vec![
        Effect::Notify(format!("asking Jira how to start {key}…")),
        Effect::Spawn(Box::pin(async move {
            let client = provider.client(&s);
            let ask = |e: anyhow::Error| format!("{e:#}");
            let me = if need_me {
                Some(match &client {
                    Ok(c) => c.myself().await.map_err(ask),
                    Err(e) => Err(format!("{e:#}")),
                })
            } else {
                None
            };
            let moves = if need_moves {
                Some(match &client {
                    Ok(c) => c.transitions(&k).await.map_err(ask),
                    Err(e) => Err(format!("{e:#}")),
                })
            } else {
                None
            };
            Box::new(JiraMsg::Started {
                site: s.name,
                key: k,
                me,
                moves,
            }) as Msg
        })),
    ]
}

/// Jira answered: open the one question, if there is anything to ask.
pub(crate) fn on_started(
    p: &mut JiraPlugin,
    site: &str,
    key: &str,
    me: Option<Result<Me, String>>,
    moves: Option<Result<Vec<Transition>, String>>,
) -> Vec<Effect> {
    p.asking.remove(&format!("start\0{site}\0{key}"));
    let mut fx = Vec::new();
    match me {
        Some(Ok(m)) if !m.account_id.is_empty() => {
            p.data.entry(site.to_string()).or_default().me = Some(m)
        }
        Some(Ok(_)) => fx.push(crate::model::failed_notice(&format!(
            "Jira did not say who you are on {site}"
        ))),
        Some(Err(e)) => fx.push(crate::model::failed_notice(&format!(
            "could not ask who you are on {site}: {e}"
        ))),
        None => {}
    }
    let moves = match moves {
        Some(Ok(ms)) => Some(ms),
        Some(Err(e)) => {
            fx.push(crate::model::failed_notice(&format!(
                "could not ask where {key} can go: {e}"
            )));
            None
        }
        None => None,
    };
    let Some(t) = join::held(p, site, key) else {
        return fx;
    };
    let me = write::me(p, site);
    let start = plan(site, t, me, moves.as_deref(), &p.cfg.start_status);
    if start.assign.is_none() && start.mv.is_none() {
        return fx;
    }
    // A background start suggestion cannot take over an editor or reviewed action.
    // The read/cache result above is still retained; no assignment or move is sent.
    if crate::write_dialog::occupied(p) {
        fx.extend(crate::write_dialog::skipped("use i or t after finishing the current action"));
        return fx;
    }
    let menu = Menu::write_plain(
        MENU,
        format!("start {key} on Jira?"),
        start.lines(),
        "start it",
        "leave it as it is",
    )
    .icon(Icon::You);
    fx.push(write::journal(p, "jira.ticket.start", site, key, false));
    p.starting = Some(start);
    fx.push(Effect::Menu(menu));
    fx
}

/// One queued move, bound to the exact accepted assignment and original scope.
#[derive(Debug)]
pub(crate) struct Pending {
    pub owner: Dispatch,
    pub change: Change,
    pub action: ActionDescriptor,
}

/// `y` or `n` at the question. `None` when the menu is not this one's.
pub(crate) fn on_menu(p: &mut JiraPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    if menu != MENU {
        return None;
    }
    let Some(start) = p.starting.take() else {
        return Some(vec![]);
    };
    if value != "yes" {
        return Some(vec![
            Effect::Report(JournalOutcome::Cancelled),
            Effect::Notify(format!("{} left as it is; nothing sent", start.key)),
        ]);
    }
    let mut changes = [start.assign, start.mv].into_iter().flatten();
    let Some(first) = changes.next() else {
        return Some(vec![]);
    };
    let mut fx = vec![Effect::Report(JournalOutcome::Confirmed)];
    // The single continuation slot is deliberately bounded: never replace an
    // already approved move, even when the new start is on a different ticket.
    if p.start_next.is_some() {
        for change in std::iter::once(first).chain(changes) {
            fx.extend([change.journal(p), Effect::Report(JournalOutcome::Blocked)]);
        }
        fx.push(Effect::Notify(
            "Jira has not finished the previous start yet; nothing sent".into(),
        ));
        return Some(fx);
    }
    let next = changes.next().map(|change| {
        let action = change.descriptor(p);
        (change, action)
    });
    let action = first.descriptor(p);
    let (sent, owner) = write::dispatch(p, first, action);
    // Put the blocked next step before the first step's result/notice, so a mode
    // refusal still says exactly why the assignment was not sent.
    if let Some((change, action)) = next {
        if let Some(owner) = owner {
            p.start_next = Some(Pending {
                owner,
                change,
                action,
            });
        } else {
            fx.extend([
                Effect::Journal(action),
                Effect::Report(JournalOutcome::Blocked),
            ]);
        }
    }
    fx.extend(sent);
    Some(fx)
}

/// Only the exact accepted assignment can release its continuation. An unrelated
/// or duplicate completion must not consume another operation's queued move.
pub(crate) fn after_wrote(
    p: &mut JiraPlugin,
    done: &Change,
    ok: bool,
    dispatch: &Dispatch,
) -> Vec<Effect> {
    if p.start_next
        .as_ref()
        .is_none_or(|next| next.owner.id != dispatch.id)
        || !matches!(done, Change::Assign { .. })
    {
        return vec![];
    }
    let next = p.start_next.take().expect("matching continuation");
    if ok && write_context::current(p, done.site(), &next.owner) {
        write::dispatch(p, next.change, next.action).0
    } else {
        let mut fx = vec![
            Effect::Journal(next.action),
            Effect::Report(JournalOutcome::Blocked),
        ];
        if ok {
            fx.push(Effect::Notify("Assignment succeeded on the original Jira connection; its move was not sent because the connection changed. Review the ticket before trying again.".into()));
        }
        fx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_core::writes;
    use crate::client::recorded::{Replay, block};
    use crate::test_support::TicketBuilder;
    use serde_json::json;

    fn tr(id: &str, to: &str, c: Category) -> Transition {
        Transition {
            id: id.into(),
            name: format!("go {to}"),
            to: to.into(),
            category: c,
        }
    }

    fn moves() -> Vec<Transition> {
        vec![
            tr("11", "To Do", Category::Todo),
            tr("31", "Done", Category::Done),
            tr("21", "In Progress", Category::Doing),
            tr("22", "In Review", Category::Doing),
        ]
    }

    fn alice() -> Me {
        Me {
            account_id: "acc-alice".into(),
            name: "Alice Example".into(),
        }
    }

    fn ticket(status: &str, c: Category, assignee: (&str, &str)) -> Ticket {
        // Planning reads only the key, status and assignment. Retain their
        // legacy values without constructing the sample plugin and its views.
        TicketBuilder::new()
            .key("ABC-12")
            .id("")
            .summary("Add a stats endpoint for the dashboard")
            .status(status, c)
            .assignee(assignee.1, assignee.0)
            .build()
    }

    mod builder_parity {
        use super::*;

        #[test]
        fn minimal_ticket_preserves_legacy_start_plans() {
            let mut original = crate::demo::sample().data["acme"].tickets["ABC-12"].clone();
            let provider_id = original.id.clone();
            assert!(!provider_id.is_empty(), "the complete demo has a provider identity");
            // This migration's legacy fixture omitted the provider ID. Keep
            // that exact starting state, and also prove that the complete
            // demo's ID does not change key-addressed assignment/move plans.
            original.id.clear();
            let me = alice();
            let ms = moves();
            for (status, category, assignee) in [
                ("To Do", Category::Todo, ("Bob Example", "acc-bob")),
                ("To Do", Category::Todo, ("", "")),
                ("In Progress", Category::Doing, ("Bob Example", "acc-bob")),
                ("To Do", Category::Todo, ("Alice Example", "acc-alice")),
                ("In Progress", Category::Doing, ("Alice Example", "acc-alice")),
            ] {
                let mut legacy = original.clone();
                legacy.status = status.into();
                legacy.category = category;
                legacy.assignee = assignee.0.into();
                legacy.assignee_id = assignee.1.into();
                let minimal = ticket(status, category, assignee);
                assert_eq!((&minimal.id, &minimal.summary), (&legacy.id, &legacy.summary));
                let mut identified = legacy.clone();
                identified.id.clone_from(&provider_id);
                for wanted in ["", "In Review"] {
                    assert_eq!(
                        plan("acme", &minimal, Some(&me), Some(&ms), wanted),
                        plan("acme", &legacy, Some(&me), Some(&ms), wanted),
                    );
                    assert_eq!(
                        plan("acme", &minimal, Some(&me), Some(&ms), wanted),
                        plan("acme", &identified, Some(&me), Some(&ms), wanted),
                        "provider IDs do not change key-addressed start plans",
                    );
                }
            }
        }
    }

    #[test]
    fn the_payload_names_the_ticket_by_key() {
        assert_eq!(
            key_of(&json!({"ticket": "ABC-12", "workspace": "/w"})).as_deref(),
            Some("ABC-12")
        );
        assert_eq!(
            key_of(&json!({"ticket": {"key": "ABC-12"}})).as_deref(),
            Some("ABC-12")
        );
        assert_eq!(key_of(&json!({"ticket": "nope"})), None);
        assert_eq!(key_of(&json!({"workspace": "/w"})), None);
    }

    #[test]
    fn the_first_move_into_progress_or_the_configured_status() {
        let ms = moves();
        assert_eq!(pick_move(&ms, "").map(|t| t.id.as_str()), Some("21"));
        assert_eq!(
            pick_move(&ms, "in review").map(|t| t.id.as_str()),
            Some("22")
        );
        assert_eq!(
            pick_move(&ms, "go In Review").map(|t| t.id.as_str()),
            Some("22")
        );
        assert!(pick_move(&ms, "Blocked").is_none());
        assert!(pick_move(&[tr("11", "To Do", Category::Todo)], "").is_none());
    }

    #[test]
    fn the_question_has_a_line_for_each_thing_it_would_change() {
        let ms = moves();
        let me = alice();
        // Both: someone else's, to do.
        let t = ticket("To Do", Category::Todo, ("Bob Example", "acc-bob"));
        let s = plan("acme", &t, Some(&me), Some(&ms), "");
        assert_eq!(
            s.lines(),
            [
                "assignee: Bob Example → you (Alice Example)",
                "status: To Do → In Progress"
            ]
        );
        // Unassigned reads as such.
        let t = ticket("To Do", Category::Todo, ("", ""));
        assert_eq!(
            plan("acme", &t, Some(&me), Some(&ms), "").lines()[0],
            "assignee: unassigned → you (Alice Example)"
        );
        // Only the assignment: already in progress.
        let t = ticket("In Progress", Category::Doing, ("Bob Example", "acc-bob"));
        let s = plan("acme", &t, Some(&me), None, "");
        assert_eq!(s.lines(), ["assignee: Bob Example → you (Alice Example)"]);
        assert!(s.mv.is_none());
        // Only the move: already yours.
        let t = ticket("To Do", Category::Todo, ("Alice Example", "acc-alice"));
        let s = plan("acme", &t, Some(&me), Some(&ms), "In Review");
        assert_eq!(s.lines(), ["status: To Do → In Review"]);
        // Nothing: yours and in progress; or no way to move it.
        let t = ticket(
            "In Progress",
            Category::Doing,
            ("Alice Example", "acc-alice"),
        );
        let s = plan("acme", &t, Some(&me), Some(&ms), "");
        assert!(s.assign.is_none() && s.mv.is_none());
        let t = ticket("To Do", Category::Todo, ("Alice Example", "acc-alice"));
        let s = plan(
            "acme",
            &t,
            Some(&me),
            Some(&[tr("11", "To Do", Category::Todo)]),
            "",
        );
        assert!(s.assign.is_none() && s.mv.is_none());
    }

    /// The sample's ABC-12 is in progress: put it back to do.
    fn todo(p: &mut JiraPlugin) {
        let t = p
            .data
            .get_mut("acme")
            .unwrap()
            .tickets
            .get_mut("ABC-12")
            .unwrap();
        t.status = "To Do".into();
        t.category = Category::Todo;
    }

    fn started_host() -> radar_core::Host {
        crate::tests::hosted()
    }

    fn plugin(host: &mut radar_core::Host) -> &mut JiraPlugin {
        crate::tests::jira_mut(host)
    }

    fn bus(host: &mut radar_core::Host, topic: &str, payload: Value) -> Vec<Effect> {
        let p = plugin(host);
        on_bus(p, topic, &payload)
    }

    #[test]
    fn other_topics_and_off_ask_nothing_and_a_known_me_that_has_it_all_asks_nothing() {
        let mut host = started_host();
        assert!(bus(&mut host, "git.started", json!({"ticket": "ABC-12"})).is_empty());
        plugin(&mut host).cfg.on_start = OnStart::Off;
        assert!(bus(&mut host, TOPIC, json!({"ticket": "ABC-12"})).is_empty());
        plugin(&mut host).cfg.on_start = OnStart::Ask;
        assert!(
            bus(&mut host, TOPIC, json!({"ticket": "ZZZ-1"})).is_empty(),
            "not a ticket here"
        );
        // In progress and yours: nothing to ask. To do: it reads what it needs, once at a time.
        assert!(bus(&mut host, TOPIC, json!({"ticket": "ABC-12"})).is_empty());
        todo(plugin(&mut host));
        let fx = bus(&mut host, TOPIC, json!({"ticket": "ABC-12"}));
        assert_eq!(
            fx.iter().filter(|e| matches!(e, Effect::Spawn(_))).count(),
            1
        );
        let again = bus(&mut host, TOPIC, json!({"ticket": "ABC-12"}));
        assert!(again.is_empty(), "already asking");
    }

    #[test]
    fn answered_it_opens_one_question_with_the_lines_and_n_sends_nothing() {
        let mut host = started_host();
        let p = plugin(&mut host);
        p.data.get_mut("acme").unwrap().me = None;
        todo(p);
        let fx = on_started(p, "acme", "ABC-12", Some(Ok(alice())), Some(Ok(moves())));
        let Some(Effect::Menu(m)) = fx.into_iter().find(|e| matches!(e, Effect::Menu(_))) else {
            panic!("no question")
        };
        assert_eq!(m.title, "start ABC-12 on Jira?");
        let notes: Vec<&str> = m
            .items
            .iter()
            .filter(|i| i.note)
            .map(|i| i.label.as_str())
            .collect();
        assert!(
            notes.last().is_some_and(|l| l.starts_with("status: ")),
            "{notes:?}"
        );
        assert!(plugin(&mut host).starting.is_some());
        let p = plugin(&mut host);
        let fx = on_menu(p, MENU, "no").unwrap();
        assert!(matches!(&fx[..], [Effect::Report(JournalOutcome::Cancelled), Effect::Notify(m)] if m.contains("nothing sent")));
        assert!(p.starting.is_none() && p.start_next.is_none());
        assert!(on_menu(p, "other", "yes").is_none());
    }

    #[test]
    fn y_sends_the_assignment_then_the_move_once_each_through_the_same_requests() {
        let mut host = started_host();
        let p = plugin(&mut host);
        todo(p);
        p.data
            .get_mut("acme")
            .unwrap()
            .tickets
            .get_mut("ABC-12")
            .unwrap()
            .assignee_id = "acc-bob".into();
        let _ = on_started(p, "acme", "ABC-12", Some(Ok(alice())), Some(Ok(moves())));
        let start = p.starting.clone().unwrap();
        let (assign, mv) = (start.assign.unwrap(), start.mv.unwrap());
        // The requests are the ones `i` and `t` make, played back.
        let r = Replay::new(vec![(204, Value::Null), (204, Value::Null)]);
        let c = Client::over(r.clone());
        for change in [&assign, &mv] {
            block(c.write(change.request().unwrap(), "ticket ABC-12")).unwrap();
        }
        let asked = r.asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert!(
            asked[0].path.ends_with("/issue/ABC-12/assignee"),
            "{}",
            asked[0].path
        );
        assert!(
            asked[1].path.ends_with("/issue/ABC-12/transitions"),
            "{}",
            asked[1].path
        );
        assert!(
            asked[0]
                .body
                .as_ref()
                .is_some_and(|b| b.to_string().contains("acc-alice"))
        );
        assert!(
            asked[1]
                .body
                .as_ref()
                .is_some_and(|b| b.to_string().contains("\"21\""))
        );
        drop(asked);
        // The actual accepted dispatch owns the move. Futures remain unpolled.
        writes::set_mode(writes::Mode::Edit, None);
        let _ = on_menu(p, MENU, "yes").unwrap();
        let owner = p.start_next.as_ref().unwrap().owner.clone();
        assert!(
            after_wrote(p, &mv, true, &owner).is_empty() && p.start_next.is_some(),
            "not the one waited for"
        );
        assert!(
            after_wrote(p, &assign, false, &owner).iter().any(|e| matches!(e, Effect::Report(JournalOutcome::Blocked))) && p.start_next.is_none(),
            "dropped"
        );
        let action = mv.descriptor(p);
        p.start_next = Some(Pending { owner: owner.clone(), change: mv, action });
        let _ = write::on_wrote(p, assign.clone(), Ok(Value::Null), &owner);
        let fx = after_wrote(p, &assign, true, &owner);
        assert!(p.start_next.is_none());
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Spawn(_))).count(), 1, "the move is sent");
    }
}
