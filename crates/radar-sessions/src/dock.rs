//! Maps sessions onto the dock's view data (`radar_core::dock::DockView`).
//!
//! Pure: what the store holds in, view data out. The host lays its own state
//! (shown, focus, height) over the result.

use std::collections::{BTreeMap, BTreeSet};

use radar_core::dock as view;
use radar_core::ui::icon::Icon;

use crate::adapter::{Capabilities, ModelInfo, ProviderSessionInfo};
use crate::event::{RadarSessionId, PermissionKind, PermissionTarget, QuestionReply};
use crate::store::{Attach, Holder, Item, PermissionState, QuestionState, Session, SessionState, Store, ToolState};

/// radar's own command that attaches a restored session ([`view::DockIntent::Resume`]), listed while it is detached.
pub const RESUME: &str = "resume";

/// Everything the dock needs besides the store.
pub struct Inputs<'a> {
    pub store: &'a Store,
    /// Tabs, in the order they were opened.
    pub order: &'a [RadarSessionId],
    pub active: Option<&'a RadarSessionId>,
    /// Open/folded items, by item id.
    pub folds: &'a BTreeMap<String, bool>,
    pub picker: Option<view::PickerKind>,
    pub history: &'a [String],
    /// Capabilities of the active session's provider.
    pub capabilities: Option<&'a Capabilities>,
    /// Models of the active session's provider (or of the default one).
    pub models: &'a [ModelInfo],
    pub chosen_model: Option<&'a str>,
    /// Sessions the providers hold, for the session picker: (provider, info).
    pub provider_sessions: &'a [(String, ProviderSessionInfo)],
    /// The terminal tabs' screens, by session id (from the adapters that run a CLI's own screen).
    pub terminals: &'a BTreeMap<String, view::TerminalView>,
    /// What the provider of the next session offers (agents, skills), and the agent chosen for it.
    pub catalog: Option<&'a crate::adapter::Catalog>,
    /// What a session of the active one's provider and folder offered after `/` last time: shown
    /// until the session itself has said.
    pub cached_commands: Option<&'a [crate::adapter::SlashCommand]>,
    pub chosen_agent: Option<&'a str>,
    /// The plugin's clock (unix seconds) and when each session's turns ran by it.
    pub now: i64,
    pub clocks: &'a BTreeMap<RadarSessionId, Clock>,
    /// Sessions whose last turn ended out of sight.
    pub unread: &'a BTreeSet<RadarSessionId>,
}

/// When a session's turn ran, at the plugin's clock (a provider's own times
/// are its own: the fake's clock counts events), and what the user was told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// The turn timed ([`crate::store::TurnMark::seq`]).
    pub seq: u64,
    pub started: i64,
    /// When it stopped running; `None` while it runs.
    pub ended: Option<i64>,
    /// It was seen running here (a turn read back from history on resume
    /// starts and ends inside one look).
    seen: bool,
    /// Its end was told.
    finished: bool,
    /// A request or a question waits, and that was told.
    waiting: bool,
}

/// What a step of a session's clock tells the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The turn ended (done, stopped or failed).
    Finished,
    /// A request or a question started to wait for the user.
    NeedsYou,
}

impl Clock {
    /// Bring the clock up to `s` at `now`; `needs_you` when something of it
    /// waits for the user (not a request a standing rule answers).
    pub fn step(&mut self, s: &Session, needs_you: bool, now: i64) -> Option<Change> {
        let t = s.turn;
        if t.seq != self.seq {
            *self = Clock { seq: t.seq, started: now, ..Clock::default() };
        }
        if t.open {
            self.ended = None;
            self.seen = true;
            self.finished = false;
        } else if self.ended.is_none() && t.seq > 0 {
            self.ended = Some(now);
        }
        let mut change = None;
        // A turn read back from history (resume) is not news.
        if t.settled() && self.ran(s) && !self.finished {
            self.finished = true;
            change = Some(Change::Finished);
        }
        if needs_you && !self.waiting {
            change = change.or(Some(Change::NeedsYou));
        }
        self.waiting = needs_you;
        change
    }

    /// The turn ran here, not read back from history: it was seen running,
    /// or something live happened in it.
    pub fn ran(&self, s: &Session) -> bool {
        self.seen || s.turn.live
    }
}

/// Something of `s` waits for the user: a question, or a request no
/// standing rule answers (`auto` holds the ones answered for them).
pub fn needs_you(s: &Session, auto: &BTreeSet<String>) -> bool {
    // Not attached: what its history shows waiting was asked by a process that is gone.
    s.attached() && (!s.pending_questions().is_empty() || s.pending().iter().any(|r| !auto.contains(&r.request_id)))
}

fn busy(s: &Session) -> bool {
    matches!(s.state, SessionState::Running | SessionState::WaitingApproval)
}

/// What the active session is doing now, or how its last turn ended; `None`
/// before its first turn, and for a turn read back from history (resume:
/// it did not run here).
pub fn activity(s: &Session, clock: Option<&Clock>, now: i64) -> Option<view::DockActivity> {
    // Restored from the last run: being read back, or waiting to be.
    let since = u32::try_from((now - s.attach_at).max(0)).unwrap_or(u32::MAX);
    let queued = u32::try_from(s.queued.len()).unwrap_or(u32::MAX);
    let detached = |doing| Some(view::DockActivity { doing, detail: s.meta.provider.to_string(), elapsed: since, tokens: None, queued });
    match s.attach {
        Attach::Resuming => return detached(view::Doing::Resuming),
        Attach::Detached => return detached(view::Doing::Detached),
        Attach::Attached => {}
    }
    let c = clock.filter(|c| c.seq > 0 && c.seq == s.turn.seq)?;
    let running = busy(s);
    if !running && !c.ran(s) {
        return None;
    }
    let to = if running { now } else { c.ended.unwrap_or(now) };
    let elapsed = u32::try_from((to - c.started).max(0)).unwrap_or(u32::MAX);
    let tokens = s.turn_tokens();
    let (doing, detail) = if s.state == SessionState::WaitingApproval {
        match s.pending().first() {
            Some(r) => (view::Doing::Approval, target_text(&r.target)),
            None => (view::Doing::Question, String::new()),
        }
    } else if running {
        let tool = s.items.iter().rev().find_map(|i| match i {
            Item::Tool { name, summary, state: ToolState::Running, .. } => Some(if summary.is_empty() { name.clone() } else { summary.clone() }),
            _ => None,
        });
        match (tool, s.items.last()) {
            (Some(t), _) => (view::Doing::Running, t),
            (None, Some(Item::Assistant { saved: false, .. })) => (view::Doing::Writing, String::new()),
            _ => (view::Doing::Thinking, String::new()),
        }
    } else {
        match s.state {
            SessionState::Stopped => (view::Doing::Stopped, String::new()),
            SessionState::Error | SessionState::Disconnected => (view::Doing::Failed, String::new()),
            _ => (view::Doing::Done, String::new()),
        }
    };
    let queued = u32::try_from(s.queued.len()).unwrap_or(u32::MAX);
    Some(view::DockActivity { doing, detail, elapsed, tokens, queued })
}

pub fn state(s: &Session) -> view::SessionState {
    match s.state {
        SessionState::Idle => view::SessionState::Idle,
        SessionState::Running => view::SessionState::Streaming,
        SessionState::WaitingApproval => view::SessionState::Approval,
        SessionState::Stopped => view::SessionState::Stopped,
        SessionState::Error | SessionState::Disconnected => view::SessionState::Error,
    }
}

fn kind_word(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Shell => "shell",
        PermissionKind::Read => "read",
        PermissionKind::Write => "write",
        PermissionKind::Url => "url",
        PermissionKind::Mcp => "mcp",
        PermissionKind::Other => "other",
    }
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// The exact target, as the provider gave it.
pub fn target_text(target: &PermissionTarget) -> String {
    match target {
        PermissionTarget::Command { command, .. } => command.clone(),
        PermissionTarget::Path { path } => path.clone(),
        PermissionTarget::Write { path, .. } => path.clone(),
        PermissionTarget::Url { url } => url.clone(),
        PermissionTarget::Mcp { server, tool, arguments } => match arguments {
            Some(args) => format!("{server}.{tool} {args}"),
            None => format!("{server}.{tool}"),
        },
        PermissionTarget::Other { description } => description.clone(),
    }
}

fn items(s: &Session, folds: &BTreeMap<String, bool>) -> Vec<view::DockItem> {
    let denied_calls: Vec<&str> = s
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Permission { request, state: PermissionState::Denied } => request.tool_call_id.as_deref(),
            _ => None,
        })
        .collect();
    let open = |id: &str| folds.get(id).copied().unwrap_or(false);
    s.items
        .iter()
        .enumerate()
        .map(|(n, item)| match item {
            Item::User { text } => view::DockItem::User { id: format!("user-{n}"), text: text.clone(), queued: None },
            Item::Assistant { message_id, text, saved } => {
                view::DockItem::assistant(message_id.clone(), text.clone(), *saved)
            }
            Item::Reasoning { message_id, text, .. } => {
                let id = format!("reasoning-{message_id}");
                view::DockItem::Reasoning { open: open(&id), id, text: text.clone() }
            }
            Item::Tool { call_id, name, summary, state, result, diff } => view::DockItem::Tool {
                diff: diff.as_deref().map(lines).unwrap_or_default(),
                id: call_id.clone(),
                title: if summary.is_empty() { name.clone() } else { summary.clone() },
                kind: name.clone(),
                state: if denied_calls.contains(&call_id.as_str()) {
                    view::ToolState::Denied
                } else {
                    match state {
                        ToolState::Running => view::ToolState::Running,
                        ToolState::Ok => view::ToolState::Done,
                        ToolState::Failed => view::ToolState::Failed,
                    }
                },
                open: open(call_id),
                result: result.as_deref().map(lines).unwrap_or_default(),
            },
            Item::Permission { request, state } => {
                // Read back while not attached: asked by a process that is gone, so it lapses with the history's end.
                let state = if !s.attached() && *state == PermissionState::Pending { &PermissionState::Lapsed } else { state };
                let lapsed = *state == PermissionState::Lapsed;
                let diff = match &request.target {
                    PermissionTarget::Write { diff: Some(d), .. } => lines(d),
                    _ => Vec::new(),
                };
                // A lapsed request has nothing to press: no answer reaches anyone.
                let mut actions = vec![];
                if !lapsed {
                    actions.push(view::PermissionAction::ApproveOnce);
                    if request.can_offer_session_approval {
                        actions.push(view::PermissionAction::ApproveForSession);
                    }
                    if !crate::settings::config::Rule::suggest(request).is_empty() {
                        actions.push(view::PermissionAction::AlwaysAllow);
                    }
                    actions.push(view::PermissionAction::Deny);
                    if !diff.is_empty() {
                        actions.push(view::PermissionAction::ViewDiff);
                    }
                }
                view::DockItem::Permission(view::DockPermission {
                    id: request.request_id.clone(),
                    kind: kind_word(request.kind).to_string(),
                    target: target_text(&request.target),
                    diff,
                    actions,
                    tool: request.tool_call_id.clone(),
                    lapsed,
                    answered: match state {
                        PermissionState::Pending | PermissionState::Lapsed => None,
                        PermissionState::Approved if s.answered_for_session(&request.request_id) => {
                            Some(view::PermissionAction::ApproveForSession)
                        }
                        PermissionState::Approved => Some(view::PermissionAction::ApproveOnce),
                        PermissionState::Denied => Some(view::PermissionAction::Deny),
                    },
                })
            }
            Item::Question { request, state, replies } => {
                let state = if !s.attached() && *state == QuestionState::Pending { &QuestionState::Lapsed } else { state };
                // While it waits, the operator's answers so far; once settled, what was answered.
                let given: &[QuestionReply] = if *state == QuestionState::Pending { s.drafts(&request.request_id) } else { replies };
                view::DockItem::Question(view::DockQuestion {
                    id: request.request_id.clone(),
                    questions: request
                        .questions
                        .iter()
                        .map(|q| {
                            let reply = given.iter().find(|r| r.question_id == q.id);
                            view::DockAsk {
                                id: q.id.clone(),
                                header: q.header.clone(),
                                text: q.text.clone(),
                                choices: q
                                    .choices
                                    .iter()
                                    .map(|c| view::DockAskChoice { label: c.label.clone(), detail: c.description.clone().unwrap_or_default() })
                                    .collect(),
                                free_text: q.free_text,
                                multiple: q.multiple,
                                answered: reply.is_some() || *state != QuestionState::Pending,
                                answer: if q.secret { Vec::new() } else { reply.map(|r| r.values.clone()).unwrap_or_default() },
                            }
                        })
                        .collect(),
                    state: match state {
                        QuestionState::Pending => view::AskState::Waiting,
                        QuestionState::Answered => view::AskState::Answered,
                        QuestionState::Declined => view::AskState::Declined,
                        QuestionState::Lapsed => view::AskState::Lapsed,
                    },
                })
            }
            Item::Error { message, .. } => view::DockItem::Error { id: format!("error-{n}"), text: message.clone() },
            Item::Notice { text } => view::DockItem::Notice { id: format!("notice-{n}"), text: text.clone() },
        })
        // What the operator sent that the provider has not shown yet: at the foot, marked.
        .chain(s.queued.iter().enumerate().map(|(n, q)| view::DockItem::User {
            id: format!("queued-{n}"),
            text: q.text.clone(),
            queued: Some(match q.holder {
                Holder::Radar => view::Queued::Here,
                Holder::Provider => view::Queued::Provider,
            }),
        }))
        .collect()
}

fn tab(s: &Session, i: &Inputs<'_>) -> view::DockTab {
    let terminals = i.terminals;
    let id = s.meta.radar_session_id.to_string();
    let unread = i.unread.contains(&s.meta.radar_session_id);
    let running = i
        .clocks
        .get(&s.meta.radar_session_id)
        .filter(|_| busy(s))
        .map(|c| u32::try_from((i.now - c.started).max(0)).unwrap_or(u32::MAX));
    let terminal = terminals.get(&id).cloned();
    // A terminal tab's state is its program's.
    let (icon, state) = match &terminal {
        Some(t) => (
            Icon::Terminal,
            match t.state {
                view::TerminalState::Running => view::SessionState::Idle,
                view::TerminalState::Exited { .. } => view::SessionState::Ended,
                view::TerminalState::Failed => view::SessionState::Error,
            },
        ),
        // Restored and not attached: never "needs you" for what its history shows waiting.
        None => (
            Icon::Bot,
            match s.attach {
                Attach::Resuming => view::SessionState::Resuming,
                Attach::Detached if s.state == SessionState::Error => view::SessionState::Error,
                Attach::Detached => view::SessionState::Detached,
                Attach::Attached => state(s),
            },
        ),
    };
    view::DockTab { id, provider: s.meta.provider.to_string(), icon: Some(icon), title: s.meta.title.clone(), state, unread, running, terminal }
}

/// The tabs alone, and the active one: what the status line's chip reads on
/// every frame, without building the transcript.
pub fn dock_tabs(i: &Inputs<'_>) -> (Vec<view::DockTab>, Option<String>) {
    let tabs = i.order.iter().filter_map(|id| i.store.get(id)).map(|s| tab(s, i)).collect();
    (tabs, i.active.filter(|id| i.store.get(id).is_some()).map(ToString::to_string))
}

/// A slash command as the composer's menu draws it: the hint, when there is one, ahead of what it does.
fn command(c: &crate::adapter::SlashCommand) -> view::DockCommand {
    let detail = match &c.argument_hint {
        Some(hint) if !c.description.is_empty() => format!("{hint}  {}", c.description),
        Some(hint) => hint.clone(),
        None => c.description.clone(),
    };
    view::DockCommand { name: c.name.trim_start_matches('/').to_string(), detail }
}

/// The dock's content. Host state (shown, focus, height) is laid over it
/// by the host.
pub fn dock_view(i: &Inputs<'_>) -> view::DockView {
    let tabs: Vec<view::DockTab> = i.order.iter().filter_map(|id| i.store.get(id)).map(|s| tab(s, i)).collect();
    let active = i.active.and_then(|id| i.store.get(id));
    let mut dock = view::DockView {
        tabs,
        active: active.map(|s| s.meta.radar_session_id.to_string()),
        ..Default::default()
    };
    dock.composer.history = i.history.to_vec();
    match active {
        None => {
            dock.composer.disabled = Some("no session: start one with a new session".into());
        }
        Some(s) => {
            dock.transcript = items(s, i.folds);
            dock.activity = activity(s, i.clocks.get(&s.meta.radar_session_id), i.now);
            let can_stop = i.capabilities.is_some_and(|c| c.stop);
            let busy = s.attached() && matches!(s.state, SessionState::Running | SessionState::WaitingApproval);
            dock.composer.can_stop = busy && can_stop;
            // A message sent while a turn runs is never refused: the provider steers with it or queues it, or radar
            // keeps it until the turn ends (shown queued at the foot of the transcript).
            dock.composer.can_send = s.connected();
            if !s.connected() {
                dock.composer.disabled = Some("disconnected from the provider".into());
            } else if s.meta.provider_session_id.as_str().is_empty() {
                dock.composer.disabled = Some("starting the session".into());
                dock.composer.can_send = false;
            }
            // What the provider reported for this session, else what its adapter always has.
            let commands = s
                .commands
                .as_deref()
                .or(i.cached_commands)
                .or_else(|| i.capabilities.map(|c| c.slash_commands.as_slice()))
                .unwrap_or_default();
            dock.composer.commands = commands.iter().map(command).collect();
            // radar's own: closing is the dock's, whatever the provider offers.
            if !dock.composer.commands.iter().any(|c| c.name == "close") {
                dock.composer.commands.push(view::DockCommand { name: "close".into(), detail: "close this session (alt-s brings it back)".into() });
            }
            // Restored and not attached: attach without sending (a message attaches it too).
            if s.attach == Attach::Detached {
                dock.composer.commands.retain(|c| c.name != RESUME);
                dock.composer.commands.push(view::DockCommand { name: RESUME.into(), detail: "attach to the provider again and read the session back (alt-r)".into() });
            }
            dock.status = view::DockStatus {
                provider: s.meta.provider.to_string(),
                model: s.meta.model.clone().unwrap_or_default(),
                agent: s.meta.agent.clone().unwrap_or_default(),
                cwd: s.meta.cwd.clone(),
                branch: None,
                usage: (s.usage.0 + s.usage.1 > 0).then(|| format!("{} tokens", s.usage.0 + s.usage.1)),
            };
        }
    }
    dock.picker = i.picker.map(|kind| view::DockPicker {
        kind,
        reading: matches!(kind, view::PickerKind::Agent | view::PickerKind::Skill) && i.catalog.is_none(),
        choices: match kind {
            view::PickerKind::Model => i
                .models
                .iter()
                .map(|m| view::DockChoice {
                    id: m.id.clone(),
                    label: m.name.clone(),
                    detail: String::new(),
                    current: i.chosen_model == Some(m.id.as_str())
                        || active.and_then(|s| s.meta.model.as_deref()) == Some(m.id.as_str()),
                })
                .collect(),
            view::PickerKind::Session => i
                .provider_sessions
                .iter()
                .map(|(provider, info)| view::DockChoice {
                    id: format!("{provider}:{}", info.provider_session_id),
                    label: info.title.clone().unwrap_or_else(|| info.provider_session_id.to_string()),
                    detail: format!("{provider} {}", info.cwd.clone().unwrap_or_default()).trim().to_string(),
                    current: active.is_some_and(|s| s.meta.provider_session_id == info.provider_session_id),
                })
                .collect(),
            // Empty until the CLI has answered: `reading` says so.
            view::PickerKind::Agent if i.catalog.is_none() => vec![],
            view::PickerKind::Agent => {
                // The CLI's own agent first, so a chosen agent can be dropped again.
                let own = view::DockChoice { id: String::new(), label: "the CLI's own agent".into(), detail: String::new(), current: i.chosen_agent.is_none() };
                std::iter::once(own)
                    .chain(i.catalog.into_iter().flat_map(|c| &c.agents).map(|a| view::DockChoice {
                        id: a.name.clone(),
                        label: a.name.clone(),
                        detail: [a.source.clone(), a.description.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                        current: i.chosen_agent == Some(a.name.as_str()),
                    }))
                    .collect()
            }
            view::PickerKind::Skill => i
                .catalog
                .into_iter()
                .flat_map(|c| &c.skills)
                .map(|s| view::DockChoice {
                    id: s.name.clone(),
                    label: s.name.clone(),
                    detail: [s.source.clone(), s.description.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                    current: false,
                })
                .collect(),
        },
    });
    dock
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::*;
    use crate::store::SessionMeta;

    fn store_with(events: Vec<EventKind>) -> (Store, RadarSessionId) {
        let radar = RadarSessionId::new("s1");
        let mut store = Store::new();
        store.insert(SessionMeta {
            radar_session_id: radar.clone(),
            provider: ProviderId::new("fake"),
            provider_session_id: ProviderSessionId::new("p1"),
            cwd: "/work".into(),
            title: "Fake".into(),
            model: Some("fake-large".into()),
            created_at: 1,
            cursor: None,
            agent: None,
            review: false,
        });
        for (n, kind) in events.into_iter().enumerate() {
            let durability = match kind {
                EventKind::AssistantDelta { .. } => Durability::Ephemeral,
                _ => Durability::Persisted,
            };
            store.ingest(Event {
                radar_session_id: radar.clone(),
                provider: ProviderId::new("fake"),
                provider_session_id: ProviderSessionId::new("p1"),
                provider_event_id: Some(ProviderEventId::new(format!("e{n}"))),
                sequence: n as u64 + 1,
                observed_at: 1,
                kind,
                durability,
                agent: None,
                parent: None,
                payload: serde_json::Value::Null,
            });
        }
        (store, radar)
    }

    static NO_TERMINALS: BTreeMap<String, view::TerminalView> = BTreeMap::new();

    fn inputs<'a>(store: &'a Store, order: &'a [RadarSessionId], caps: &'a Capabilities, folds: &'a BTreeMap<String, bool>) -> Inputs<'a> {
        Inputs {
            store,
            order,
            active: order.first(),
            folds,
            picker: None,
            history: &[],
            capabilities: Some(caps),
            models: &[],
            chosen_model: None,
            provider_sessions: &[],
            terminals: &NO_TERMINALS,
            catalog: None,
            cached_commands: None,
            chosen_agent: None,
            now: 0,
            clocks: &NO_CLOCKS,
            unread: &NO_UNREAD,
        }
    }

    static NO_CLOCKS: BTreeMap<RadarSessionId, Clock> = BTreeMap::new();
    static NO_UNREAD: BTreeSet<RadarSessionId> = BTreeSet::new();

    fn ev(store: &mut Store, radar: &RadarSessionId, seq: u64, kind: EventKind) {
        let durability = match kind {
            EventKind::AssistantDelta { .. } | EventKind::Usage { .. } | EventKind::Idle => Durability::Ephemeral,
            _ => Durability::Persisted,
        };
        store.ingest(Event {
            radar_session_id: radar.clone(),
            provider: ProviderId::new("fake"),
            provider_session_id: ProviderSessionId::new("p1"),
            provider_event_id: Some(ProviderEventId::new(format!("x{seq}"))),
            sequence: seq,
            // The provider's own clock, which the activity line never reads.
            observed_at: 7,
            kind,
            durability,
            agent: None,
            parent: None,
            payload: serde_json::Value::Null,
        });
    }

    /// The activity line at `now`, after stepping the session's clock there.
    fn line(store: &Store, radar: &RadarSessionId, clocks: &mut BTreeMap<RadarSessionId, Clock>, now: i64) -> (Option<view::DockActivity>, Option<Change>) {
        let s = store.get(radar).unwrap();
        let change = clocks.entry(radar.clone()).or_default().step(s, needs_you(s, &BTreeSet::new()), now);
        (activity(s, clocks.get(radar), now), change)
    }

    #[test]
    fn the_activity_line_says_what_the_turn_does_for_how_long_and_how_it_ended() {
        use view::Doing::*;
        let (mut store, radar) = store_with(vec![]);
        let mut clocks = BTreeMap::new();
        let act = |a: Option<view::DockActivity>| a.map(|a| (a.doing, a.detail, a.elapsed, a.tokens));
        assert_eq!(line(&store, &radar, &mut clocks, 100), (None, None), "no turn yet");
        ev(&mut store, &radar, 10, EventKind::UserMessage { text: "go".into() });
        ev(&mut store, &radar, 11, EventKind::TurnStarted);
        assert_eq!(act(line(&store, &radar, &mut clocks, 100).0), Some((Thinking, String::new(), 0, None)));
        ev(&mut store, &radar, 12, EventKind::AssistantDelta { message_id: "m1".into(), text: "Look".into() });
        ev(&mut store, &radar, 13, EventKind::Usage { input_tokens: Some(1000), output_tokens: Some(200) });
        assert_eq!(act(line(&store, &radar, &mut clocks, 104).0), Some((Writing, String::new(), 4, Some(1200))));
        ev(&mut store, &radar, 14, EventKind::ToolStarted { call_id: "c1".into(), name: "shell".into(), summary: "git diff".into(), diff: None });
        assert_eq!(act(line(&store, &radar, &mut clocks, 110).0), Some((Running, "git diff".into(), 10, Some(1200))));
        let request = PermissionRequest {
            request_id: "r1".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "git push".into(), segments: vec![] },
            tool_call_id: None,
            can_offer_session_approval: false,
        };
        ev(&mut store, &radar, 15, EventKind::PermissionRequested { request });
        let (a, change) = line(&store, &radar, &mut clocks, 120);
        assert_eq!(act(a), Some((Approval, "git push".into(), 20, Some(1200))));
        assert_eq!(change, Some(Change::NeedsYou), "said once");
        assert_eq!(line(&store, &radar, &mut clocks, 121).1, None);
        ev(&mut store, &radar, 16, EventKind::PermissionResolved { request_id: "r1".into(), approved: true });
        ev(&mut store, &radar, 17, EventKind::ToolCompleted { call_id: "c1".into(), ok: true, summary: "ok".into() });
        // Copilot ends a turn per model call: the turn carries on until the session is idle.
        ev(&mut store, &radar, 18, EventKind::TurnEnded);
        assert_eq!(line(&store, &radar, &mut clocks, 125).1, None, "not finished yet");
        ev(&mut store, &radar, 19, EventKind::TurnStarted);
        ev(&mut store, &radar, 20, EventKind::Usage { input_tokens: Some(300), output_tokens: Some(100) });
        assert_eq!(act(line(&store, &radar, &mut clocks, 130).0), Some((Thinking, String::new(), 30, Some(1600))), "the same turn");
        ev(&mut store, &radar, 21, EventKind::TurnEnded);
        ev(&mut store, &radar, 22, EventKind::Idle);
        let (a, change) = line(&store, &radar, &mut clocks, 142);
        assert_eq!(act(a), Some((Done, String::new(), 42, Some(1600))));
        assert_eq!(change, Some(Change::Finished));
        // Idle, the line keeps how long the last turn took.
        assert_eq!(act(line(&store, &radar, &mut clocks, 500).0), Some((Done, String::new(), 42, Some(1600))));
        // The next turn counts its own time and tokens.
        ev(&mut store, &radar, 23, EventKind::UserMessage { text: "again".into() });
        ev(&mut store, &radar, 24, EventKind::Aborted);
        let (a, change) = line(&store, &radar, &mut clocks, 503);
        assert_eq!(act(a), None, "read back from history: started and stopped inside one look, nothing live");
        assert_eq!(change, None, "and not news");
        // A turn that ran live inside one look (the session went idle) is.
        ev(&mut store, &radar, 29, EventKind::UserMessage { text: "quick".into() });
        ev(&mut store, &radar, 30, EventKind::TurnStarted);
        ev(&mut store, &radar, 31, EventKind::Idle);
        let (a, change) = line(&store, &radar, &mut clocks, 505);
        assert_eq!((act(a), change), (Some((Done, String::new(), 0, None)), Some(Change::Finished)));
        ev(&mut store, &radar, 32, EventKind::UserMessage { text: "once more".into() });
        line(&store, &radar, &mut clocks, 510);
        ev(&mut store, &radar, 33, EventKind::Error { message: "lost".into(), retryable: true });
        let (a, change) = line(&store, &radar, &mut clocks, 519);
        assert_eq!(act(a), Some((Failed, String::new(), 9, None)));
        assert_eq!(change, Some(Change::Finished));
    }

    #[test]
    fn a_question_waits_for_your_answer_and_a_request_a_rule_answers_never_needs_you() {
        let (mut store, radar) = store_with(vec![]);
        ev(&mut store, &radar, 1, EventKind::UserMessage { text: "go".into() });
        let request = PermissionRequest {
            request_id: "r1".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "ls".into(), segments: vec![] },
            tool_call_id: None,
            can_offer_session_approval: false,
        };
        ev(&mut store, &radar, 2, EventKind::PermissionRequested { request });
        let auto: BTreeSet<String> = ["r1".to_string()].into();
        let s = store.get(&radar).unwrap();
        assert!(!needs_you(s, &auto) && needs_you(s, &BTreeSet::new()));
        ev(&mut store, &radar, 3, EventKind::PermissionResolved { request_id: "r1".into(), approved: true });
        let ask = QuestionRequest {
            request_id: "q1".into(),
            questions: vec![Question { id: "0".into(), header: None, text: "which?".into(), choices: vec![], free_text: true, multiple: false, secret: false }],
            tool_call_id: None,
        };
        ev(&mut store, &radar, 4, EventKind::QuestionAsked { request: ask });
        let s = store.get(&radar).unwrap();
        assert!(needs_you(s, &auto));
        let mut clocks = BTreeMap::new();
        clocks.insert(radar.clone(), Clock::default());
        assert_eq!(activity(s, None, 9), None);
        let (a, _) = line(&store, &radar, &mut clocks, 9);
        assert_eq!(a.map(|a| a.doing), Some(view::Doing::Question));
    }

    #[test]
    fn tabs_say_unread_and_how_long_they_run_without_the_transcript() {
        let (mut store, radar) = store_with(vec![]);
        ev(&mut store, &radar, 1, EventKind::TurnStarted);
        let mut clocks = BTreeMap::new();
        clocks.entry(radar.clone()).or_insert_with(Clock::default).step(store.get(&radar).unwrap(), false, 50);
        let unread: BTreeSet<RadarSessionId> = [radar.clone()].into();
        let caps = Capabilities::default();
        let folds = BTreeMap::new();
        let order = [radar.clone()];
        let i = Inputs { now: 57, clocks: &clocks, unread: &unread, ..inputs(&store, &order, &caps, &folds) };
        let (tabs, active) = dock_tabs(&i);
        assert_eq!((tabs[0].running, tabs[0].unread, active.as_deref()), (Some(7), true, Some("s1")));
        assert_eq!(dock_view(&i).tabs, tabs);
        ev(&mut store, &radar, 2, EventKind::Idle);
        let i = Inputs { now: 60, clocks: &clocks, unread: &unread, ..inputs(&store, &order, &caps, &folds) };
        assert_eq!(dock_tabs(&i).0[0].running, None, "only while it runs");
    }

    #[test]
    fn streamed_text_is_unsaved_and_an_approval_waits_with_its_exact_command() {
        let request = PermissionRequest {
            request_id: "r1".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "ls -la".into(), segments: vec![] },
            tool_call_id: Some("c1".into()),
            can_offer_session_approval: false,
        };
        let (store, radar) = store_with(vec![
            EventKind::AssistantDelta { message_id: "m1".into(), text: "Look".into() },
            EventKind::ToolStarted { call_id: "c1".into(), name: "shell".into(), summary: "ls -la".into(), diff: None },
            EventKind::PermissionRequested { request },
        ]);
        let caps = Capabilities { stop: true, ..Default::default() };
        let folds = BTreeMap::new();
        let order = [radar];
        let dock = dock_view(&inputs(&store, &order, &caps, &folds));
        assert_eq!(dock.tabs[0].state, view::SessionState::Approval);
        assert!(matches!(&dock.transcript[0], view::DockItem::Assistant { saved: false, .. }));
        match &dock.transcript[2] {
            view::DockItem::Permission(p) => {
                assert_eq!(p.target, "ls -la");
                assert_eq!(p.answered, None);
                assert_eq!(p.actions, vec![view::PermissionAction::ApproveOnce, view::PermissionAction::AlwaysAllow, view::PermissionAction::Deny]);
            }
            other => panic!("{other:?}"),
        }
        assert!(dock.composer.can_stop);
        assert!(dock.composer.can_send, "a message can be sent while the turn waits: it is queued, never refused");
    }

    #[test]
    fn messages_on_their_way_follow_the_transcript_marked_by_who_holds_them() {
        use crate::store::Holder;
        let (mut store, radar) = store_with(vec![EventKind::UserMessage { text: "go".into() }, EventKind::TurnStarted]);
        store.get_mut(&radar).unwrap().queue("and?".into(), Holder::Provider);
        store.get_mut(&radar).unwrap().queue("then this".into(), Holder::Radar);
        let caps = Capabilities::default();
        let folds = BTreeMap::new();
        let order = [radar];
        let dock = dock_view(&inputs(&store, &order, &caps, &folds));
        let users: Vec<_> = dock
            .transcript
            .iter()
            .map(|i| match i {
                view::DockItem::User { text, queued, .. } => (text.as_str(), *queued),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(users, vec![("go", None), ("and?", Some(view::Queued::Provider)), ("then this", Some(view::Queued::Here))]);
    }

    #[test]
    fn approve_for_session_is_offered_only_where_the_request_allows_it_and_shows_once_chosen() {
        let request = |id: &str, can: bool| PermissionRequest {
            request_id: id.into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "ls".into(), segments: vec![] },
            tool_call_id: None,
            can_offer_session_approval: can,
        };
        let (mut store, radar) = store_with(vec![
            EventKind::PermissionRequested { request: request("r1", true) },
            EventKind::PermissionResolved { request_id: "r1".into(), approved: true },
            EventKind::PermissionRequested { request: request("r2", true) },
            EventKind::PermissionRequested { request: request("r3", false) },
        ]);
        store.get_mut(&radar).unwrap().approve_for_session(&request("r1", true));
        let caps = Capabilities::default();
        let folds = BTreeMap::new();
        let order = [radar];
        let dock = dock_view(&inputs(&store, &order, &caps, &folds));
        let perms: Vec<&view::DockPermission> =
            dock.transcript.iter().filter_map(|i| if let view::DockItem::Permission(p) = i { Some(p) } else { None }).collect();
        assert_eq!(perms[0].answered, Some(view::PermissionAction::ApproveForSession));
        use view::PermissionAction::{AlwaysAllow, ApproveForSession, ApproveOnce, Deny};
        assert_eq!(perms[1].actions, vec![ApproveOnce, ApproveForSession, AlwaysAllow, Deny]);
        assert_eq!(perms[2].actions, vec![ApproveOnce, AlwaysAllow, Deny]);
    }

    #[test]
    fn a_denied_request_marks_its_tool_denied_and_the_folds_come_from_the_host() {
        let request = PermissionRequest {
            request_id: "r1".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "rm x".into(), segments: vec![] },
            tool_call_id: Some("c1".into()),
            can_offer_session_approval: false,
        };
        let (store, radar) = store_with(vec![
            EventKind::ToolStarted { call_id: "c1".into(), name: "shell".into(), summary: "rm x".into(), diff: None },
            EventKind::PermissionRequested { request },
            EventKind::PermissionResolved { request_id: "r1".into(), approved: false },
            EventKind::ToolCompleted { call_id: "c1".into(), ok: false, summary: "denied".into() },
        ]);
        let caps = Capabilities::default();
        let mut folds = BTreeMap::new();
        folds.insert("c1".to_string(), true);
        let order = [radar];
        let dock = dock_view(&inputs(&store, &order, &caps, &folds));
        match &dock.transcript[0] {
            view::DockItem::Tool { state, open, result, .. } => {
                assert_eq!(*state, view::ToolState::Denied);
                assert!(open);
                assert_eq!(result, &vec!["denied".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&dock.transcript[1], view::DockItem::Permission(p) if p.answered == Some(view::PermissionAction::Deny)));
    }

    #[test]
    fn a_session_that_reported_commands_shows_them_in_place_of_the_adapters_with_their_descriptions() {
        use crate::adapter::SlashCommand;
        let (mut store, radar) = store_with(vec![EventKind::TurnEnded]);
        let caps = Capabilities { slash_commands: vec![SlashCommand::new("model", "choose the model")], ..Default::default() };
        let folds = BTreeMap::new();
        let order = [radar.clone()];
        let shown = |store: &Store| dock_view(&inputs(store, &order, &caps, &folds)).composer.commands;
        // radar's own `/close` follows whatever the provider offers.
        let close = view::DockCommand { name: "close".into(), detail: "close this session (alt-s brings it back)".into() };
        assert_eq!(shown(&store), vec![view::DockCommand { name: "model".into(), detail: "choose the model".into() }, close.clone()]);

        store.get_mut(&radar).unwrap().commands = Some(vec![
            SlashCommand::new("/compact", "summarise the conversation"),
            SlashCommand::new("review", "review a change").with_hint("<pr>"),
            SlashCommand::new("clean", "").with_hint("[path]"),
        ]);
        assert_eq!(
            shown(&store),
            vec![
                view::DockCommand { name: "compact".into(), detail: "summarise the conversation".into() },
                view::DockCommand { name: "review".into(), detail: "<pr>  review a change".into() },
                view::DockCommand { name: "clean".into(), detail: "[path]".into() },
                close,
            ]
        );
    }

    #[test]
    fn with_no_session_the_composer_says_why_it_is_off() {
        let store = Store::new();
        let folds = BTreeMap::new();
        let dock = dock_view(&Inputs {
            store: &store,
            order: &[],
            active: None,
            folds: &folds,
            picker: None,
            history: &[],
            capabilities: None,
            models: &[],
            chosen_model: None,
            provider_sessions: &[],
            terminals: &BTreeMap::new(),
            catalog: None,
            cached_commands: None,
            chosen_agent: None,
            now: 0,
            clocks: &BTreeMap::new(),
            unread: &BTreeSet::new(),
        });
        assert!(dock.tabs.is_empty());
        assert!(dock.composer.disabled.is_some());
        assert!(!dock.composer.can_send);
    }
}
