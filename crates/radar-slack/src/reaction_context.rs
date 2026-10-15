//! Session-local, one-use reaction choices and operation-owned completion.
//! No payload or presentation text enters snapshots or action metadata.

use std::collections::BTreeMap;

use radar_core::journal::{ActionDescriptor, Origin, Outcome};
use radar_core::plugin::{Effect, Menu, MenuItem};
use radar_core::writes;

use crate::client::Posted;
use crate::read_context::ReadStamp;
use crate::{SlackPlugin, Write, page};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReactionPayload {
    pub channel: String,
    pub ts: String,
    pub name: String,
    /// The original refresh destination, never reconstructed from newer data.
    pub thread: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReactionOperation {
    pub id: u64,
    pub read: ReadStamp,
    pub payload: ReactionPayload,
}

#[derive(Debug)]
struct Target {
    read: ReadStamp,
    channel: String,
    ts: String,
    thread: String,
    alias: String,
    whose: String,
    action: ActionDescriptor,
}

#[derive(Debug)]
enum Intent {
    Pick(Target),
    Send {
        read: ReadStamp,
        payload: ReactionPayload,
    },
}

#[derive(Debug)]
struct ActiveReaction {
    operation: ReactionOperation,
    claimed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ReactionContext {
    next_intent: u64,
    issued: u64,
    pending: Option<(String, Intent)>,
    active: BTreeMap<u64, ActiveReaction>,
}

fn next(counter: &mut u64) -> u64 {
    *counter = counter
        .checked_add(1)
        .expect("Slack reaction sequence exhausted");
    *counter
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
        observation("slack.reaction.intent.skipped"),
        Effect::Notify("that reaction choice is no longer current; open it again".into()),
    ]
}

impl SlackPlugin {
    fn reaction_intent(&mut self, kind: &str, intent: Intent) -> String {
        let id = format!("{kind}:{}", next(&mut self.reaction_context.next_intent));
        self.reaction_context.pending = Some((id.clone(), intent));
        id
    }

    pub(crate) fn pick_reaction(&mut self, channel: &str, ts: &str, whose: String) -> Vec<Effect> {
        if !self.authenticated() {
            return vec![
                Effect::Report(Outcome::Blocked),
                Effect::Notify("check the Slack connection before reacting".into()),
            ];
        }
        let Effect::Journal(action) = self.journal_write("slack.reaction.add", channel, Some(ts))
        else {
            unreachable!("reaction descriptor")
        };
        let target = Target {
            read: self.read_stamp(),
            channel: channel.into(),
            ts: ts.into(),
            thread: self.thread_of(channel, ts),
            alias: self.alias(channel),
            whose,
            action,
        };
        let title = format!("react to {}", target.whose);
        let id = self.reaction_intent("react", Intent::Pick(target));
        let items = page::REACTIONS
            .iter()
            .map(|(key, name, says)| MenuItem::new(*key, format!(":{name}:"), *says, *name))
            .collect();
        vec![Effect::Menu(Menu::new(id, title, items))]
    }

    pub(crate) fn answer_reaction(&mut self, menu: &str, value: &str) -> Option<Vec<Effect>> {
        if !menu.starts_with("react:") && !menu.starts_with("react-send:") {
            return None;
        }
        if !self
            .reaction_context
            .pending
            .as_ref()
            .is_some_and(|(id, _)| id == menu)
        {
            return Some(changed_choice());
        }
        let (_, intent) = self
            .reaction_context
            .pending
            .take()
            .expect("matching reaction intent");
        Some(match intent {
            Intent::Pick(target) => {
                if !self.authenticated()
                    || !self.is_current_read(target.read)
                    || !page::REACTIONS.iter().any(|(_, name, _)| *name == value)
                {
                    return Some(changed_choice());
                }
                let journal = Effect::Journal(target.action);
                let title = format!("react :{value}: to {}?", target.whose);
                let lines = vec![format!(
                    "adds :{value}: to that message in #{}",
                    target.alias
                )];
                let id = self.reaction_intent(
                    "react-send",
                    Intent::Send {
                        read: target.read,
                        payload: ReactionPayload {
                            channel: target.channel,
                            ts: target.ts,
                            name: value.into(),
                            thread: target.thread,
                        },
                    },
                );
                vec![
                    journal,
                    Effect::Menu(
                        Menu::write_plain(id, title, lines, "add it", "leave it as it is").adds(),
                    ),
                ]
            }
            Intent::Send { read, payload } => {
                if !self.authenticated() || !self.is_current_read(read) {
                    return Some(changed_choice());
                }
                if value != "yes" {
                    return Some(vec![
                        Effect::Report(Outcome::Cancelled),
                        Effect::Notify("nothing sent".into()),
                    ]);
                }
                let operation = ReactionOperation {
                    id: next(&mut self.reaction_context.issued),
                    read,
                    payload,
                };
                self.reaction_context.active.insert(
                    operation.id,
                    ActiveReaction {
                        operation: operation.clone(),
                        claimed: false,
                    },
                );
                self.write(Write::React { operation })
            }
        })
    }

    /// Claim once before client construction, including synchronous refusals.
    pub(crate) fn claim_reaction(&mut self, operation: &ReactionOperation) -> bool {
        if !self.authenticated() || !self.is_current_read(operation.read) {
            return false;
        }
        let Some(active) = self.reaction_context.active.get_mut(&operation.id) else {
            return false;
        };
        if active.claimed || active.operation != *operation {
            return false;
        }
        active.claimed = true;
        true
    }

    pub(crate) fn finish_reaction(
        &mut self,
        operation: ReactionOperation,
        result: Result<Posted, String>,
        outcome: Outcome,
    ) -> Vec<Effect> {
        let mut fx = vec![Effect::Report(outcome)];
        let Some(active) = self.reaction_context.active.get(&operation.id) else {
            fx.push(observation(
                if operation.id != 0 && operation.id <= self.reaction_context.issued {
                    "slack.reaction.completion.duplicate"
                } else {
                    "slack.reaction.completion.unmatched"
                },
            ));
            return fx;
        };
        if !active.claimed || active.operation != operation {
            fx.push(observation("slack.reaction.completion.unmatched"));
            return fx;
        }
        self.reaction_context.active.remove(&operation.id);
        if !self.authenticated() || !self.is_current_read(operation.read) {
            fx.push(observation("slack.reaction.apply.skipped.stale"));
            fx.push(match result {
                Ok(_) => writes::done("earlier reaction added; the current Slack context was not refreshed"),
                Err(_) => writes::failed("the earlier reaction could not be confirmed; the current Slack context was left unchanged"),
            });
            return fx;
        }
        match result {
            Ok(_) => {
                fx.push(writes::done(&format!(
                    "reacted :{}:",
                    operation.payload.name
                )));
                fx.extend(page::fetch_thread(
                    self,
                    &operation.payload.channel,
                    &operation.payload.thread,
                ));
            }
            Err(error) if outcome == Outcome::Failed => fx.push(crate::view::failed(&error)),
            Err(error) => {
                let doing = self.doing(&Write::React { operation });
                fx.push(writes::failed(&crate::SERVICE.did_not(&doing, &error)));
            }
        }
        fx
    }
}
