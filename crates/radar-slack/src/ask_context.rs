//! Transient ownership of review-ask previews and sends. Frozen text never enters
//! journal metadata or persistent state. Completed IDs need no growing set.

use std::collections::BTreeMap;

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason};
use radar_core::plugin::{Effect, Menu, MenuItem};
use radar_core::writes;

use crate::ask::{self, Draft};
use crate::cache::AccountOwner;
use crate::client::Posted;
use crate::read_context::ReadStamp;
use crate::{SlackPlugin, Write};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DraftStamp {
    pub incarnation: u64,
    pub revision: u64,
}

impl DraftStamp {
    pub(crate) fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("Slack draft revision exhausted");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskPayload {
    pub channel: String,
    pub text: String,
    pub test: bool,
    pub mrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskOperation {
    pub owner: AccountOwner,
    pub id: u64,
    pub draft: DraftStamp,
    pub read: ReadStamp,
    pub payload: AskPayload,
}

#[derive(Debug)]
struct PreparedAsk {
    id: u64,
    draft: DraftStamp,
    read: ReadStamp,
    payload: AskPayload,
}

#[derive(Debug)]
struct DraftIntent {
    id: u64,
    draft: DraftStamp,
    read: ReadStamp,
}

#[derive(Debug)]
enum AskIntent {
    Send(PreparedAsk),
    Drop(DraftIntent),
    Channel(DraftIntent),
}

impl AskIntent {
    fn key(&self) -> (&'static str, u64) {
        match self {
            Self::Send(p) => ("ask-send", p.id),
            Self::Drop(p) => ("ask-drop", p.id),
            Self::Channel(p) => ("ask-channel", p.id),
        }
    }
}

#[derive(Debug)]
struct ActiveAsk {
    operation: AskOperation,
    claimed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct AskContext {
    origin: Option<(u64, ReadStamp, Option<AccountOwner>)>,
    next_draft: u64,
    next_intent: u64,
    issued: u64,
    pending: Option<AskIntent>,
    active: BTreeMap<u64, ActiveAsk>,
}

fn next(counter: &mut u64) -> u64 {
    *counter = counter
        .checked_add(1)
        .expect("Slack ask sequence exhausted");
    *counter
}

fn observation(kind: &'static str) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("slack".into()));
    action.metadata.count = Some(1);
    action.metadata.reason = Some(Reason::Unspecified);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

impl SlackPlugin {
    pub(crate) fn begin_ask_draft(&mut self, mut draft: Draft) {
        draft.stamp.incarnation = next(&mut self.ask_context.next_draft);
        self.ask_context.origin = Some((
            draft.stamp.incarnation,
            self.read_stamp(),
            self.session_owner.clone().filter(|_| self.authenticated()),
        ));
        self.draft = Some(draft);
    }

    fn ensure_ask_stamp(&mut self) {
        let read = self.read_stamp();
        if let Some(draft) = self.draft.as_mut()
            && draft.stamp.incarnation == 0
        {
            draft.stamp.incarnation = next(&mut self.ask_context.next_draft);
            self.ask_context.origin = Some((draft.stamp.incarnation, read, None));
        }
    }

    /// An authored draft never adopts a different account or token generation.
    pub(crate) fn ask_draft_blocked(&self) -> Option<&'static str> {
        let draft = self.draft.as_ref()?;
        if !self.authenticated() {
            return Some("check the Slack connection before posting; your draft is kept");
        }
        if !self
            .ask_context
            .origin
            .as_ref()
            .is_some_and(|(incarnation, read, owner)| {
                *incarnation == draft.stamp.incarnation
                    && self.is_current_read(*read)
                    && owner.is_some()
                    && owner.as_ref() == self.session_owner.as_ref()
            })
        {
            return Some(
                "this draft belongs to an earlier or unverified Slack context; copy its words, then start a new draft",
            );
        }
        None
    }

    /// Recover authored words without resolving mentions using another account.
    pub(crate) fn copy_ask_draft(&self) -> Vec<Effect> {
        let Some(draft) = &self.draft else {
            return vec![];
        };
        let text = draft
            .items
            .iter()
            .enumerate()
            .map(|(row, item)| {
                let words = if row == draft.row {
                    draft
                        .edit
                        .as_ref()
                        .map(|edit| edit.value())
                        .unwrap_or_else(|| item.words.clone())
                } else {
                    item.words.clone()
                };
                let mut lines = vec![format!("{} {}", item.label, item.url), words];
                lines.extend(item.tickets.iter().map(|(key, url)| format!("{key} {url}")));
                lines.join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        vec![
            Effect::Copy(text),
            Effect::Notify("review ask words copied; the draft is kept".into()),
        ]
    }

    fn ask_payload(&self) -> Option<AskPayload> {
        let draft = self.draft.as_ref()?;
        if draft.items.is_empty()
            || ask::check_template(&self.cfg.review.template, &self.data.groups).is_err()
        {
            return None;
        }
        let channel = self.cfg.channel(&draft.channel)?;
        Some(AskPayload {
            channel: channel.id.clone(),
            text: ask::text(&self.cfg, &self.data.groups, &draft.channel, &draft.items),
            test: channel.kind == crate::config::Kind::Test
                || channel.alias == self.cfg.review.test_channel,
            mrs: draft
                .items
                .iter()
                .map(|item| (item.id.clone(), item.url.clone()))
                .collect(),
        })
    }

    pub(crate) fn preview_ask(&mut self) -> Vec<Effect> {
        // A new preview attempt supersedes even an older dismissed dialog.
        self.ask_context.pending = None;
        self.ensure_ask_stamp();
        let Some(draft) = self.draft.as_ref() else {
            return vec![];
        };
        if let Some(why) = self.ask_draft_blocked() {
            return vec![
                observation("slack.review_ask.draft.preview.skipped"),
                Effect::Report(Outcome::Blocked),
                Effect::Notify(format!("not yet: {why}")),
            ];
        }
        let channel = self
            .cfg
            .channel(&draft.channel)
            .map(|c| c.id.as_str())
            .unwrap_or_default();
        let journal = self.journal_write("slack.review_ask.post", channel, None);
        if let Some(why) = draft.blocked(&self.cfg, &self.data.groups) {
            return vec![
                journal,
                Effect::Report(Outcome::Blocked),
                Effect::Notify(format!("not yet: {why}")),
            ];
        }
        if draft.edit.is_some() {
            return vec![
                journal,
                Effect::Report(Outcome::Blocked),
                Effect::Notify("finish editing before previewing the review ask".into()),
            ];
        }
        let Some(payload) = self.ask_payload() else {
            return vec![journal, Effect::Report(Outcome::Blocked)];
        };
        let id = next(&mut self.ask_context.next_intent);
        let menu = Menu::write_confirm(
            format!("ask-send:{id}"),
            format!("post this in #{}?", draft.channel),
            &payload.text,
            "post it",
            "drop it",
        );
        self.ask_context.pending = Some(AskIntent::Send(PreparedAsk {
            id,
            draft: draft.stamp,
            read: self.read_stamp(),
            payload,
        }));
        vec![journal, Effect::Menu(menu)]
    }

    fn draft_intent(&mut self) -> Option<DraftIntent> {
        self.ask_context.pending = None;
        self.ensure_ask_stamp();
        let draft = self.draft.as_ref()?;
        if draft.edit.is_some() {
            return None;
        }
        Some(DraftIntent {
            id: next(&mut self.ask_context.next_intent),
            draft: draft.stamp,
            read: self.read_stamp(),
        })
    }

    pub(crate) fn preview_ask_drop(&mut self) -> Vec<Effect> {
        let Some(intent) = self.draft_intent() else {
            return vec![];
        };
        let menu = Menu::confirm(
            format!("ask-drop:{}", intent.id),
            "drop this review ask?",
            "drop it",
            "keep it",
        )
        .saying("only this draft is dropped; already confirmed sends are unaffected");
        self.ask_context.pending = Some(AskIntent::Drop(intent));
        vec![Effect::Menu(menu)]
    }

    pub(crate) fn preview_ask_channel(&mut self) -> Vec<Effect> {
        let Some(intent) = self.draft_intent() else {
            return vec![];
        };
        let draft = self.draft.as_ref();
        let items = self
            .cfg
            .channels
            .iter()
            .map(|channel| {
                let mut hint = channel.kind.word().to_string();
                if channel.alias == self.cfg.review.channel {
                    hint = "the review channel: the group is mentioned".into();
                } else if channel.alias == self.cfg.review.test_channel {
                    hint = "the test channel: no one is mentioned".into();
                }
                if draft.is_some_and(|d| d.channel == channel.alias) {
                    hint = format!("{hint} · now");
                }
                MenuItem::pick(format!("#{}", channel.alias), hint, channel.alias.clone())
            })
            .collect();
        let menu = Menu::new(
            format!("ask-channel:{}", intent.id),
            "post the review ask in",
            items,
        );
        self.ask_context.pending = Some(AskIntent::Channel(intent));
        vec![Effect::Menu(menu)]
    }

    pub(crate) fn answer_ask(&mut self, menu: &str, value: &str) -> Option<Vec<Effect>> {
        let (kind, number) = menu.split_once(':')?;
        if !matches!(kind, "ask-send" | "ask-drop" | "ask-channel") {
            return None;
        }
        let id = number.parse::<u64>().ok();
        if !self.ask_context.pending.as_ref().is_some_and(|pending| {
            let (expected_kind, expected_id) = pending.key();
            kind == expected_kind && id == Some(expected_id)
        }) {
            return Some(vec![
                Effect::Report(Outcome::Blocked),
                Effect::Notify("that review-ask choice is no longer current; open it again".into()),
            ]);
        }
        let intent = self.ask_context.pending.take().expect("matching intent");
        match intent {
            AskIntent::Send(prepared) => Some(self.answer_ask_send(prepared, value)),
            AskIntent::Drop(prepared) | AskIntent::Channel(prepared) => {
                if kind == "ask-drop" && value != "yes" {
                    return Some(vec![]);
                }
                let current = self.is_current_read(prepared.read)
                    && self
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.stamp == prepared.draft && draft.edit.is_none());
                if !current {
                    return Some(vec![
                        Effect::Report(Outcome::Blocked),
                        observation("slack.review_ask.draft.intent.skipped"),
                        Effect::Notify("the review ask changed; open that choice again".into()),
                    ]);
                }
                if kind == "ask-drop" {
                    self.draft = None;
                    if matches!(self.pages.last(), Some(crate::page::Page::Draft)) {
                        self.pages.pop();
                    }
                    Some(vec![Effect::Notify("review ask draft dropped".into())])
                } else if self.cfg.channel(value).is_none() {
                    Some(vec![
                        Effect::Report(Outcome::Blocked),
                        Effect::Notify(
                            "that channel is no longer available; choose it again".into(),
                        ),
                    ])
                } else {
                    let draft = self.draft.as_mut().expect("matching draft");
                    if draft.channel != value {
                        draft.channel = value.into();
                        draft.changed();
                    }
                    Some(vec![])
                }
            }
        }
    }

    fn answer_ask_send(&mut self, prepared: PreparedAsk, value: &str) -> Vec<Effect> {
        if value != "yes" {
            let mut effects = vec![Effect::Report(Outcome::Cancelled)];
            if value != "edit" {
                effects.push(Effect::Notify(format!("nothing sent{}", writes::kept("p"))));
            }
            return effects;
        }
        let current = self.draft.as_ref().is_some_and(|draft| {
            draft.stamp == prepared.draft && draft.edit.is_none() && draft.sending.is_none()
        }) && self.ask_draft_blocked().is_none()
            && self.is_current_read(prepared.read)
            && self.ask_payload().as_ref() == Some(&prepared.payload);
        if !current {
            return vec![
                Effect::Report(Outcome::Blocked),
                Effect::Notify("the review ask changed; preview it again before posting".into()),
            ];
        }
        let operation = AskOperation {
            owner: self.session_owner.clone().expect("verified draft owner"),
            id: next(&mut self.ask_context.issued),
            draft: prepared.draft,
            read: prepared.read,
            payload: prepared.payload,
        };
        self.ask_context.active.insert(
            operation.id,
            ActiveAsk {
                operation: operation.clone(),
                claimed: false,
            },
        );
        if let Some(draft) = self.draft.as_mut() {
            draft.sending = Some(operation.id);
            draft.error = None;
        }
        self.write(Write::Ask { operation })
    }

    /// Claim an accepted intent once, including attempts refused synchronously.
    pub(crate) fn claim_ask(&mut self, operation: &AskOperation) -> bool {
        if !self.authenticated()
            || !self.is_current_read(operation.read)
            || self.session_owner.as_ref() != Some(&operation.owner)
        {
            return false;
        }
        let Some(active) = self.ask_context.active.get_mut(&operation.id) else {
            return false;
        };
        if active.claimed || active.operation != *operation {
            return false;
        }
        active.claimed = true;
        true
    }

    pub(crate) fn duplicate_ask(&self, operation: &AskOperation) -> bool {
        operation.id != 0
            && operation.id <= self.ask_context.issued
            && !self.ask_context.active.contains_key(&operation.id)
    }

    pub(crate) fn duplicate_ask_effects(&self, outcome: Outcome) -> Vec<Effect> {
        vec![
            Effect::Report(outcome),
            observation("slack.review_ask.completion.duplicate"),
        ]
    }

    pub(crate) fn finish_ask(
        &mut self,
        operation: AskOperation,
        result: Result<Posted, String>,
        outcome: Outcome,
    ) -> Vec<Effect> {
        if self.duplicate_ask(&operation) {
            return self.duplicate_ask_effects(outcome);
        }
        let known = self
            .ask_context
            .active
            .get(&operation.id)
            .is_some_and(|active| active.claimed && active.operation == operation);
        if !known {
            return vec![Effect::Report(outcome), observation("slack.review_ask.completion.unmatched"),
                Effect::Notify("an earlier review-ask result could not be associated; the current draft was left unchanged".into())];
        }
        self.ask_context.active.remove(&operation.id);
        let owns = self.draft.as_ref().is_some_and(|draft| {
            draft.stamp.incarnation == operation.draft.incarnation
                && draft.sending == Some(operation.id)
        });
        let applies = owns
            && self.authenticated()
            && self.session_owner.as_ref() == Some(&operation.owner)
            && self.is_current_read(operation.read)
            && self
                .draft
                .as_ref()
                .is_some_and(|draft| draft.stamp == operation.draft && draft.edit.is_none())
            && self.ask_payload().as_ref() == Some(&operation.payload);
        if owns {
            self.draft.as_mut().expect("owned draft").sending = None;
        }
        let mut effects = vec![Effect::Report(outcome)];
        if !applies {
            effects.push(observation("slack.review_ask.draft.apply.skipped"));
        }
        match result {
            Ok(posted) => {
                self.remember_asked(
                    &operation.owner,
                    crate::model::Asked {
                        mrs: operation.payload.mrs,
                        channel: if posted.channel.is_empty() {
                            operation.payload.channel
                        } else {
                            posted.channel
                        },
                        ts: posted.ts,
                        test: operation.payload.test,
                        replies: 0,
                        reactions: vec![],
                    },
                );
                if applies {
                    self.draft = None;
                    if matches!(self.pages.last(), Some(crate::page::Page::Draft)) {
                        self.pages.pop();
                    }
                }
                effects.push(Effect::PersistCache);
                effects.push(writes::done(if applies {
                    "review asked · its thread is followed on the merge request"
                } else {
                    "earlier review ask posted · the current draft was left unchanged"
                }));
            }
            Err(error) => {
                if applies {
                    self.draft.as_mut().expect("matching draft").error = Some(error.clone());
                    let doing = format!(
                        "post the review ask in #{}",
                        self.alias(&operation.payload.channel)
                    );
                    effects.push(writes::failed(&format!(
                        "{}{}",
                        crate::SERVICE.did_not(&doing, &error),
                        writes::kept("p")
                    )));
                } else {
                    effects.push(writes::failed("the earlier review ask could not be confirmed; the current draft was left unchanged"));
                }
            }
        }
        effects
    }
}
