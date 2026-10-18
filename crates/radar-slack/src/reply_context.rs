//! Session-local ownership and bounded retention of reply text. Neither frozen
//! payloads nor kept replies are serialized into cache or action history.
use std::collections::BTreeMap;

use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::{Effect, Menu, MenuItem};
use radar_core::ui::composer::Composer;
use radar_core::writes;

use crate::ask_context::DraftStamp;
use crate::client::Posted;
use crate::read_context::ReadStamp;
use crate::{Replying, SlackPlugin, Write};

pub(crate) const MAX_KEPT_REPLIES: usize = 32;
pub(crate) const MAX_KEPT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyStatus {
    Unsent,
    Uncertain,
}
impl ReplyStatus {
    fn words(self) -> &'static str {
        match self {
            Self::Unsent => "not sent",
            Self::Uncertain => "outcome uncertain: a post may already exist",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplyPayload {
    pub channel: String,
    pub thread: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplyOperation {
    pub id: u64,
    pub draft: DraftStamp,
    pub read: ReadStamp,
    pub payload: ReplyPayload,
    pub original: String,
    pub team: String,
    pub status: ReplyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeptReply {
    pub id: u64,
    pub channel: String,
    pub thread: String,
    pub original: String,
    pub team: String,
    pub read: ReadStamp,
    pub status: ReplyStatus,
}

#[derive(Debug)]
struct PreparedReply {
    draft: DraftStamp,
    read: ReadStamp,
    payload: ReplyPayload,
    original: String,
}
#[derive(Debug)]
enum Intent {
    Send(PreparedReply),
    Drop(PreparedReply),
    List {
        entries: Vec<u64>,
        active: Vec<u64>,
        allow_open: bool,
        target: Option<(String, String)>,
        read: ReadStamp,
        composer: Option<PreparedReply>,
    },
    Entry {
        entry: u64,
        allow_open: bool,
        composer: Option<PreparedReply>,
    },
    Remove(u64),
    Active(u64),
}
#[derive(Debug)]
struct ActiveReply {
    operation: ReplyOperation,
    claimed: bool,
}
#[derive(Debug, Default)]
pub(crate) struct ReplyContext {
    next_draft: u64,
    next_intent: u64,
    next_kept: u64,
    issued: u64,
    pending: Option<(String, Intent)>,
    active: BTreeMap<u64, ActiveReply>,
    pub kept: BTreeMap<u64, KeptReply>,
}
fn next(n: &mut u64) -> u64 {
    *n = n.checked_add(1).expect("Slack reply sequence exhausted");
    *n
}
fn observation(kind: &'static str) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("slack".into()));
    action.metadata.count = Some(1);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}
fn changed_choice() -> Vec<Effect> {
    vec![
        Effect::Report(Outcome::Blocked),
        observation("slack.reply.draft.intent.skipped"),
        Effect::Notify("that reply choice is no longer current; open it again".into()),
    ]
}
fn full() -> Vec<Effect> {
    vec![Effect::Notify("kept replies are full; your text is still here · ctrl-r to copy or drop kept text before continuing".into())]
}
impl ReplyContext {
    /// Active operations reserve their exact original UTF-8 bytes until they
    /// either succeed or become kept replies. The editable composer is separate.
    pub(crate) fn usage(&self) -> (usize, usize) {
        (
            self.kept.len() + self.active.len(),
            self.kept.values().map(|r| r.original.len()).sum::<usize>()
                + self
                    .active
                    .values()
                    .map(|a| a.operation.original.len())
                    .sum::<usize>(),
        )
    }
    fn room(&self, text: &str, removing: Option<u64>) -> bool {
        let (mut count, mut bytes) = self.usage();
        if let Some(r) = removing.and_then(|id| self.kept.get(&id)) {
            count -= 1;
            bytes -= r.original.len();
        }
        count < MAX_KEPT_REPLIES && text.len() <= MAX_KEPT_BYTES.saturating_sub(bytes)
    }
    fn keep(&mut self, mut reply: KeptReply) {
        reply.id = next(&mut self.next_kept);
        self.kept.insert(reply.id, reply);
    }
}
impl SlackPlugin {
    pub(crate) fn reply_editor_blocked(&self) -> bool {
        self.mapping.is_some()
            || self.filter_input.is_some()
            || self.edit.is_some()
            || self.draft.as_ref().is_some_and(|d| d.edit.is_some())
    }
    fn reply_intent(&mut self, kind: &'static str, intent: Intent) -> String {
        let id = format!("{kind}:{}", next(&mut self.reply_context.next_intent));
        self.reply_context.pending = Some((id.clone(), intent));
        id
    }
    fn prepared_reply(&self) -> Option<PreparedReply> {
        let r = self.replying.as_ref()?;
        let original = r.composer.text();
        Some(PreparedReply {
            draft: r.stamp,
            read: self.read_stamp(),
            payload: ReplyPayload {
                channel: r.channel.clone(),
                thread: r.thread.clone(),
                text: original.trim_end().into(),
            },
            original,
        })
    }
    fn matches_reply(&self, p: &PreparedReply) -> bool {
        self.is_current_read(p.read)
            && self.replying.as_ref().is_some_and(|r| {
                r.stamp == p.draft
                    && r.channel == p.payload.channel
                    && r.thread == p.payload.thread
                    && r.composer.text() == p.original
            })
    }
    fn matches_composer(&self, prepared: Option<&PreparedReply>) -> bool {
        match prepared {
            Some(p) => self.matches_reply(p),
            None => self.replying.is_none(),
        }
    }
    fn keep_composer(&mut self) {
        if let Some(r) = self.replying.take() {
            let original = r.composer.text();
            if !original.is_empty() {
                self.reply_context.keep(KeptReply {
                    id: 0,
                    channel: r.channel,
                    thread: r.thread,
                    original,
                    team: r.team,
                    read: r.read,
                    status: r.status,
                });
            }
        }
    }
    fn can_keep_composer(&self, removing: Option<u64>) -> bool {
        self.replying.as_ref().is_none_or(|r| {
            let text = r.composer.text();
            text.is_empty() || self.reply_context.room(&text, removing)
        })
    }
    fn set_reply(&mut self, channel: &str, thread: &str, text: &str, status: ReplyStatus) {
        self.replying = Some(Replying {
            channel: channel.into(),
            thread: thread.into(),
            composer: Composer::new(text),
            stamp: DraftStamp {
                incarnation: next(&mut self.reply_context.next_draft),
                revision: 0,
            },
            read: self.read_stamp(),
            team: self
                .data
                .me
                .as_ref()
                .map(|m| m.team_id.clone())
                .unwrap_or_default(),
            status,
        });
    }
    /// A seeded re-ask is a new composer; save existing work before replacing it.
    pub(crate) fn begin_reply(
        &mut self,
        channel: &str,
        thread: &str,
        seed: Option<&str>,
    ) -> Vec<Effect> {
        self.reply_context.pending = None;
        if self.reply_editor_blocked() {
            return vec![Effect::Notify(
                "finish the other editor before opening a reply; kept text is unchanged".into(),
            )];
        }
        if seed.is_none()
            && self.replying.as_ref().is_some_and(|r| {
                r.channel == channel && r.thread == thread && self.is_current_read(r.read)
            })
        {
            return vec![];
        }
        if !self.can_keep_composer(None) {
            return full();
        }
        self.keep_composer();
        self.set_reply(
            channel,
            thread,
            seed.unwrap_or_default(),
            ReplyStatus::Unsent,
        );
        vec![]
    }
    pub(crate) fn start_reply(&mut self, channel: &str, thread: &str) -> Vec<Effect> {
        if self.replying.as_ref().is_some_and(|r| {
            r.channel == channel && r.thread == thread && self.is_current_read(r.read)
        }) {
            return vec![];
        }
        if self
            .reply_context
            .kept
            .values()
            .any(|r| r.channel == channel && r.thread == thread)
            || self.reply_context.active.values().any(|a| {
                a.operation.payload.channel == channel && a.operation.payload.thread == thread
            })
        {
            return self.kept_reply_list(Some((channel.into(), thread.into())));
        }
        self.begin_reply(channel, thread, None)
    }
    pub(crate) fn preview_reply(&mut self) -> Vec<Effect> {
        self.reply_context.pending = None;
        let Some(prepared) = self.prepared_reply() else {
            return vec![];
        };
        if prepared.payload.text.trim().is_empty() {
            return vec![Effect::Notify(
                "nothing to post yet: write the reply".into(),
            )];
        }
        let r = self.replying.as_ref().expect("prepared composer");
        if !self.authenticated() || !self.is_current_read(r.read) {
            return vec![observation("slack.reply.draft.preview.skipped"), Effect::Report(Outcome::Blocked), Effect::Notify("this reply belongs to an earlier Slack context; ctrl-r can keep and copy its text".into())];
        }
        let journal = self.journal_write(
            "slack.reply.post",
            &prepared.payload.channel,
            Some(&prepared.payload.thread),
        );
        let title = format!(
            "post this in {}?",
            self.reply_place(&prepared.payload.channel, &prepared.payload.thread)
        );
        let text = prepared.payload.text.clone();
        let uncertain = r.status == ReplyStatus::Uncertain;
        let id = self.reply_intent("reply-send", Intent::Send(prepared));
        let mut menu = Menu::write_confirm(id, title, &text, "post it", "drop it");
        if uncertain {
            menu = menu
                .saying("an earlier post may already exist; check Slack before posting this again");
        }
        vec![journal, Effect::Menu(menu)]
    }
    pub(crate) fn preview_reply_drop(&mut self) -> Vec<Effect> {
        self.reply_context.pending = None;
        let Some(prepared) = self.prepared_reply() else {
            return vec![];
        };
        let id = self.reply_intent("reply-drop", Intent::Drop(prepared));
        vec![Effect::Menu(
            Menu::confirm(id, "drop this reply draft?", "drop it", "keep it")
                .saying("only this draft is dropped; already confirmed sends are unaffected"),
        )]
    }
    pub(crate) fn kept_replies(&mut self) -> Vec<Effect> {
        self.kept_reply_list(None)
    }
    /// Settings route keys to section editors, so never create an off-screen
    /// reply composer there. Copy/drop still work without configured credentials.
    pub(crate) fn kept_replies_in_settings(&mut self) -> Vec<Effect> {
        self.kept_reply_list_with(None, false)
    }
    fn kept_reply_list(&mut self, target: Option<(String, String)>) -> Vec<Effect> {
        self.kept_reply_list_with(target, true)
    }
    fn kept_reply_list_with(
        &mut self,
        target: Option<(String, String)>,
        allow_open: bool,
    ) -> Vec<Effect> {
        self.reply_context.pending = None;
        let entries: Vec<u64> = self
            .reply_context
            .kept
            .values()
            .filter(|r| {
                target
                    .as_ref()
                    .is_none_or(|(c, t)| &r.channel == c && &r.thread == t)
            })
            .map(|r| r.id)
            .collect();
        let mut items: Vec<MenuItem> = entries
            .iter()
            .map(|id| {
                let r = &self.reply_context.kept[id];
                MenuItem::pick(
                    format!(
                        "{} · {} / {}",
                        if r.team.is_empty() {
                            "unknown workspace"
                        } else {
                            &r.team
                        },
                        r.channel,
                        r.thread
                    ),
                    r.status.words(),
                    format!("entry:{id}"),
                )
            })
            .collect();
        let active: Vec<u64> = self
            .reply_context
            .active
            .values()
            .filter(|a| {
                target.as_ref().is_none_or(|(c, t)| {
                    &a.operation.payload.channel == c && &a.operation.payload.thread == t
                })
            })
            .map(|a| a.operation.id)
            .collect();
        for id in &active {
            let r = &self.reply_context.active[id].operation;
            items.push(MenuItem::pick(
                format!(
                    "{} · {} / {}",
                    if r.team.is_empty() {
                        "unknown workspace"
                    } else {
                        &r.team
                    },
                    r.payload.channel,
                    r.payload.thread
                ),
                "send pending; outcome unknown; do not retry",
                format!("active:{id}"),
            ));
        }
        if target.is_some() {
            items.push(MenuItem::new(
                'n',
                "new reply",
                "write a separate reply",
                "new",
            ));
        }
        if self.replying.is_some() {
            items.push(MenuItem::new(
                'y',
                "copy current draft",
                "leave the composer unchanged",
                "copy-current",
            ));
            items.push(MenuItem::new(
                's',
                "stay with current draft",
                "retain the open composer here",
                "keep",
            ));
        }
        if items.is_empty() {
            return vec![Effect::Notify("no kept replies in this session".into())];
        }
        let id = self.reply_intent(
            "reply-kept",
            Intent::List {
                entries,
                active,
                allow_open,
                target,
                read: self.read_stamp(),
                composer: self.prepared_reply(),
            },
        );
        vec![Effect::Menu(Menu::new(
            id,
            "kept replies · this session",
            items,
        ))]
    }
    fn active_reply_entry(&mut self, operation: u64) -> Vec<Effect> {
        let Some(active) = self.reply_context.active.get(&operation) else {
            return vec![Effect::Notify(
                "that send has completed; open kept replies again for its retained text".into(),
            )];
        };
        let r = &active.operation;
        let title = format!(
            "pending reply · {} · {} / {}",
            if r.team.is_empty() {
                "unknown workspace"
            } else {
                &r.team
            },
            r.payload.channel,
            r.payload.thread
        );
        let mut items: Vec<MenuItem> = radar_core::plugin::note_lines(&r.original, 72)
            .into_iter()
            .map(MenuItem::note)
            .collect();
        items.push(MenuItem::note(
            "send pending; outcome unknown; do not retry",
        ));
        items.push(MenuItem::new(
            'y',
            "copy",
            "keep the active reservation unchanged",
            "copy",
        ));
        let id = self.reply_intent("reply-pending-entry", Intent::Active(operation));
        vec![Effect::Menu(Menu::new(id, title, items))]
    }
    fn kept_reply_entry(&mut self, entry: u64, allow_open: bool) -> Vec<Effect> {
        let Some(r) = self.reply_context.kept.get(&entry) else {
            return changed_choice();
        };
        let title = format!(
            "kept reply · {} · {} / {}",
            if r.team.is_empty() {
                "unknown workspace"
            } else {
                &r.team
            },
            r.channel,
            r.thread
        );
        let mut items: Vec<MenuItem> = radar_core::plugin::note_lines(&r.original, 72)
            .into_iter()
            .map(MenuItem::note)
            .collect();
        items.push(MenuItem::note(r.status.words()));
        if allow_open && self.is_current_read(r.read) {
            items.push(MenuItem::new(
                'e',
                "edit",
                "open this text; nothing is sent",
                "edit",
            ));
        } else {
            items.push(MenuItem::note(if !self.is_current_read(r.read) {
                "earlier Slack context: copy the text to a freshly chosen destination"
            } else {
                "view, copy or drop here; open the Slack screen to edit kept text"
            }));
        }
        items.push(MenuItem::new('y', "copy", "keep a copy here too", "copy"));
        items.push(MenuItem::new('d', "drop", "remove this kept copy…", "drop"));
        let id = self.reply_intent(
            "reply-kept-entry",
            Intent::Entry {
                entry,
                allow_open,
                composer: self.prepared_reply(),
            },
        );
        vec![Effect::Menu(Menu::new(id, title, items))]
    }
    fn open_kept_reply(&mut self, entry: u64) -> Vec<Effect> {
        if self.reply_editor_blocked() {
            return vec![Effect::Notify(
                "finish the other editor before opening a reply; kept text is unchanged".into(),
            )];
        }
        let Some(r) = self.reply_context.kept.get(&entry) else {
            return changed_choice();
        };
        if !self.is_current_read(r.read) {
            return changed_choice();
        }
        if !self.can_keep_composer(Some(entry)) {
            return full();
        }
        let r = self
            .reply_context
            .kept
            .remove(&entry)
            .expect("checked entry");
        self.keep_composer();
        self.set_reply(&r.channel, &r.thread, &r.original, r.status);
        vec![Effect::Notify(if r.status == ReplyStatus::Uncertain {
            "reply recovered; an earlier post may already exist; check Slack before posting again"
        } else { "reply recovered; nothing sent" }.into())]
    }
    pub(crate) fn answer_reply(&mut self, menu: &str, value: &str) -> Option<Vec<Effect>> {
        if ![
            "reply-send:",
            "reply-drop:",
            "reply-kept:",
            "reply-kept-entry:",
            "reply-kept-drop:",
            "reply-pending-entry:",
        ]
        .iter()
        .any(|p| menu.starts_with(p))
        {
            return None;
        }
        if !self
            .reply_context
            .pending
            .as_ref()
            .is_some_and(|(id, _)| id == menu)
        {
            return Some(changed_choice());
        }
        let (_, intent) = self
            .reply_context
            .pending
            .take()
            .expect("matching reply intent");
        Some(match intent {
            Intent::Send(p) => {
                if !self.matches_reply(&p) {
                    return Some(changed_choice());
                }
                if value == "edit" {
                    return Some(vec![Effect::Report(Outcome::Cancelled)]);
                }
                if value != "yes" {
                    if !self.can_keep_composer(None) {
                        let mut fx = vec![Effect::Report(Outcome::Cancelled)];
                        fx.extend(full());
                        return Some(fx);
                    }
                    let uncertain = self
                        .replying
                        .as_ref()
                        .is_some_and(|r| r.status == ReplyStatus::Uncertain);
                    self.keep_composer();
                    return Some(vec![
                        Effect::Report(Outcome::Cancelled),
                        Effect::Notify(if uncertain {
                            "nothing sent by this confirmation; an earlier post may already exist · c has your text"
                        } else { "nothing sent · c has your text" }.into()),
                    ]);
                }
                if !self.authenticated()
                    || !self
                        .replying
                        .as_ref()
                        .is_some_and(|r| self.is_current_read(r.read))
                {
                    return Some(changed_choice());
                }
                if !self.reply_context.room(&p.original, None) {
                    let mut fx = vec![Effect::Report(Outcome::Blocked)];
                    fx.extend(full());
                    return Some(fx);
                }
                let r = self.replying.take().expect("matching composer");
                let operation = ReplyOperation {
                    id: next(&mut self.reply_context.issued),
                    draft: p.draft,
                    read: p.read,
                    payload: p.payload,
                    original: p.original,
                    team: self
                        .data
                        .me
                        .as_ref()
                        .map(|m| m.team_id.clone())
                        .unwrap_or(r.team),
                    status: r.status,
                };
                self.reply_context.active.insert(
                    operation.id,
                    ActiveReply {
                        operation: operation.clone(),
                        claimed: false,
                    },
                );
                self.write(Write::Reply { operation })
            }
            Intent::Drop(p) => {
                if !self.matches_reply(&p) {
                    return Some(changed_choice());
                }
                if value == "yes" {
                    self.replying = None;
                    vec![Effect::Notify(
                        "reply draft dropped; confirmed sends are unaffected".into(),
                    )]
                } else {
                    vec![]
                }
            }
            Intent::List {
                entries,
                active,
                allow_open,
                target,
                read,
                composer,
            } => {
                if value == "copy-current" {
                    if !composer.as_ref().is_some_and(|p| self.matches_reply(p)) {
                        return Some(changed_choice());
                    }
                    vec![
                        Effect::Copy(composer.expect("matching composer").original),
                        Effect::Notify("reply draft copied; the composer is unchanged".into()),
                    ]
                } else if value == "keep" {
                    if !self.is_current_read(read)
                        || !composer.as_ref().is_some_and(|p| self.matches_reply(p))
                    {
                        return Some(changed_choice());
                    }
                    if !self.can_keep_composer(None) {
                        return Some(full());
                    }
                    self.keep_composer();
                    self.kept_reply_list_with(None, allow_open)
                } else if value == "new" {
                    if !allow_open
                        || !self.is_current_read(read)
                        || !self.matches_composer(composer.as_ref())
                    {
                        return Some(changed_choice());
                    }
                    match target {
                        Some((c, t)) => self.begin_reply(&c, &t, None),
                        None => changed_choice(),
                    }
                } else if let Some(id) = value
                    .strip_prefix("entry:")
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|id| entries.contains(id))
                {
                    self.kept_reply_entry(id, allow_open)
                } else if let Some(id) = value
                    .strip_prefix("active:")
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|id| active.contains(id))
                {
                    self.active_reply_entry(id)
                } else {
                    vec![]
                }
            }
            Intent::Active(operation) => {
                if value == "copy" {
                    self.reply_context.active.get(&operation).map(|a| vec![Effect::Copy(a.operation.original.clone()),
                        Effect::Notify("pending reply copied; its outcome is unknown; do not retry".into())])
                        .unwrap_or_else(|| vec![Effect::Notify("that send has completed; open kept replies again for its retained text".into())])
                } else {
                    vec![]
                }
            }
            Intent::Entry {
                entry,
                allow_open,
                composer,
            } => {
                match value {
                    "edit" => {
                        if !allow_open || !self.matches_composer(composer.as_ref()) {
                            return Some(changed_choice());
                        }
                        self.open_kept_reply(entry)
                    }
                    "copy" => self
                        .reply_context
                        .kept
                        .get(&entry)
                        .map(|r| {
                            vec![
                                Effect::Copy(r.original.clone()),
                                Effect::Notify(
                                    "kept reply copied; its retained copy is unchanged".into(),
                                ),
                            ]
                        })
                        .unwrap_or_else(changed_choice),
                    "drop" => {
                        if !self.reply_context.kept.contains_key(&entry) {
                            return Some(changed_choice());
                        }
                        let id = self.reply_intent("reply-kept-drop", Intent::Remove(entry));
                        vec![Effect::Menu(Menu::confirm(id, "drop this kept reply?", "drop it", "keep it").saying("only this retained copy is removed; confirmed sends are unaffected"))]
                    }
                    _ => vec![],
                }
            }
            Intent::Remove(entry) => {
                if value == "yes" && self.reply_context.kept.remove(&entry).is_some() {
                    vec![Effect::Notify("kept reply dropped".into())]
                } else {
                    vec![]
                }
            }
        })
    }
    pub(crate) fn claim_reply(&mut self, operation: &ReplyOperation) -> bool {
        if !self.authenticated() || !self.is_current_read(operation.read) {
            return false;
        }
        let Some(active) = self.reply_context.active.get_mut(&operation.id) else {
            return false;
        };
        if active.claimed || active.operation != *operation {
            return false;
        }
        active.claimed = true;
        true
    }
    pub(crate) fn duplicate_reply(&self, operation: &ReplyOperation) -> bool {
        operation.id != 0
            && operation.id <= self.reply_context.issued
            && !self.reply_context.active.contains_key(&operation.id)
    }
    pub(crate) fn duplicate_reply_effects(&self, outcome: Outcome) -> Vec<Effect> {
        vec![
            Effect::Report(outcome),
            observation("slack.reply.completion.duplicate"),
        ]
    }
    pub(crate) fn finish_reply(
        &mut self,
        operation: ReplyOperation,
        result: Result<Posted, String>,
        outcome: Outcome,
    ) -> Vec<Effect> {
        if self.duplicate_reply(&operation) {
            return self.duplicate_reply_effects(outcome);
        }
        if !self
            .reply_context
            .active
            .get(&operation.id)
            .is_some_and(|a| a.claimed && a.operation == operation)
        {
            return vec![
                Effect::Report(outcome),
                observation("slack.reply.completion.unmatched"),
                Effect::Notify(
                    "an earlier reply result could not be associated; retained text is unchanged"
                        .into(),
                ),
            ];
        }
        self.reply_context.active.remove(&operation.id);
        let mut fx = vec![Effect::Report(outcome)];
        match result {
            Ok(_) => {
                if self.authenticated() && self.is_current_read(operation.read) {
                    fx.push(writes::done(&format!(
                        "reply posted in {}",
                        self.reply_place(&operation.payload.channel, &operation.payload.thread)
                    )));
                    fx.extend(crate::page::fetch_thread(
                        self,
                        &operation.payload.channel,
                        &operation.payload.thread,
                    ));
                } else {
                    fx.push(observation("slack.reply.refresh.skipped.stale"));
                    fx.push(writes::done(
                        "earlier reply posted; the current Slack context was not refreshed",
                    ));
                }
            }
            Err(_) => {
                let status = if outcome == Outcome::Unknown {
                    ReplyStatus::Uncertain
                } else {
                    operation.status
                };
                self.reply_context.keep(KeptReply {
                    id: 0,
                    channel: operation.payload.channel,
                    thread: operation.payload.thread,
                    original: operation.original,
                    team: operation.team,
                    read: operation.read,
                    status,
                });
                fx.push(writes::failed(if status == ReplyStatus::Uncertain {
                    "reply outcome uncertain; a post may already exist · c has your text · ctrl-r lists kept replies"
                } else { "reply not sent · c has your text · ctrl-r lists kept replies" }));
            }
        }
        fx
    }
}
