//! Writing to a merge request from its page: a comment on a line of the
//! diff or on the merge request, a reply to a thread, resolving a thread
//! (or opening it again), approving (or taking the approval back). As
//! Jira's writes: each is shown exactly and asked first, nothing is sent
//! before `y`, it is sent once and never again whatever comes back, and
//! what GitLab said is shown on the status line and in the page's "can it
//! merge?" panel. Text GitLab refused, or that was declined at the
//! question, is kept: `c` in the same place has it back.
//! Comments and replies add (write mode); resolving, approving, rebase
//! and merge change what is there (edit mode). While radar's mode refuses one
//! its question says so and `y` sends nothing. The question, the mode and
//! the words are the kit's (`radar_core::writes`).
//!
//! Comments are typed in a small composer under the line (or at the foot
//! of the page): `⏎` starts a new line, `ctrl-s` shows what will be posted
//! and asks, `esc` stops (asking first when there is text).

use radar_core::design::help_bar::KeyHint;
use radar_core::journal::Outcome as JournalOutcome;
use radar_core::plugin::{Effect, Menu, Msg, Outcome};
use radar_core::view::Chord;
use radar_core::ui::composer::{self, Composer, Typed};
use radar_core::writes::{self, Service};
use radar_core::log;
use serde_json::{Value, json};

use crate::client::{Client, Method, Req};
use crate::comment_recovery::{self, Destination, Review, Status};
use crate::model::DiffRefs;
use crate::mr_actions::{Act, Acting};
use crate::pages::mr_key;
use crate::GitlabPlugin;

/// GitLab, as writes name it.
pub(crate) const SERVICE: Service = Service::new("GitLab");

/// An unknown remote outcome is not proof that a write did not execute.
pub(crate) fn failure_words(what: &str, error: &str, outcome: JournalOutcome) -> String {
    if outcome == JournalOutcome::Unknown {
        format!(
            "Could not confirm the result of asking GitLab to {what}: {error} Check GitLab before trying again."
        )
    } else {
        SERVICE.did_not(what, error)
    }
}

const CONFIRM_MENU: &str = "gl-write";
const DROP_MENU: &str = "gl-drop";

/// A line of a diff a comment is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineAt {
    pub new_path: String,
    pub old_path: String,
    /// Kept lines have both; added lines a new one; removed lines an old one.
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub refs: DiffRefs,
}

impl LineAt {
    /// `StatsController.java line 21` (`old line 12` for a removed one).
    pub fn words(&self) -> String {
        let file = self.new_path.rsplit('/').next().unwrap_or(&self.new_path);
        match (self.new, self.old) {
            (Some(n), _) => format!("{file} line {n}"),
            (None, Some(o)) => format!("{file} old line {o}"),
            (None, None) => file.to_string(),
        }
    }
}

/// A change to a merge request, built and waiting for its `y`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    /// A comment on the merge request itself.
    Note {
        project: String,
        iid: u64,
        text: String,
    },
    /// A new thread on a line of the diff.
    Line {
        project: String,
        iid: u64,
        at: LineAt,
        text: String,
    },
    /// A reply in a thread (by its id), which is at `place`.
    Reply {
        project: String,
        iid: u64,
        discussion: String,
        place: String,
        text: String,
    },
    /// Resolve a thread, or open it again.
    Resolve {
        project: String,
        iid: u64,
        discussion: String,
        place: String,
        resolve: bool,
    },
    /// Approve the merge request (the commit `sha`, when known), or take it back.
    Approve {
        project: String,
        iid: u64,
        approve: bool,
        sha: Option<String>,
    },
}

impl Change {
    /// Stable intent only: text, file paths and commit contents never enter the journal.
    fn journal(&self, p: &GitlabPlugin) -> Effect {
        let operation = match self {
            Self::Note { .. } => "gitlab.comment.post",
            Self::Line { .. } => "gitlab.line_comment.post",
            Self::Reply { .. } => "gitlab.reply.post",
            Self::Resolve { resolve: true, .. } => "gitlab.thread.resolve",
            Self::Resolve { resolve: false, .. } => "gitlab.thread.reopen",
            Self::Approve { approve: true, .. } => "gitlab.approval.add",
            Self::Approve { approve: false, .. } => "gitlab.approval.remove",
        };
        let (project, iid) = self.mr();
        crate::mr_actions::journal(p, operation, project, iid)
    }

    pub(crate) fn mr(&self) -> (&str, u64) {
        match self {
            Self::Note { project, iid, .. }
            | Self::Line { project, iid, .. }
            | Self::Reply { project, iid, .. }
            | Self::Resolve { project, iid, .. }
            | Self::Approve { project, iid, .. } => (project, *iid),
        }
    }

    pub fn key(&self) -> String {
        let (project, iid) = self.mr();
        mr_key(project, iid)
    }

    /// The word it is recorded under on the page: `comment`, `approve`, …
    pub fn what(&self) -> &'static str {
        match self {
            Self::Note { .. } | Self::Line { .. } => "comment",
            Self::Reply { .. } => "reply",
            Self::Resolve { resolve: true, .. } => "resolve",
            Self::Resolve { resolve: false, .. } => "reopen",
            Self::Approve { approve: true, .. } => "approve",
            Self::Approve { approve: false, .. } => "unapprove",
        }
    }

    /// Whether it adds (a comment, a reply) or changes what is there.
    pub fn kind(&self) -> writes::Change {
        match self {
            Self::Note { .. } | Self::Line { .. } | Self::Reply { .. } => writes::Change::Add,
            Self::Resolve { .. } | Self::Approve { .. } => writes::Change::Edit,
        }
    }

    /// What it does, in words: `post a comment on !263`.
    pub fn doing(&self) -> String {
        let (_, iid) = self.mr();
        match self {
            Self::Note { .. } => format!("post a comment on !{iid}"),
            Self::Line { at, .. } => format!("post a comment on {}", at.words()),
            Self::Reply { place, .. } => format!("post a reply in the thread on {place}"),
            Self::Resolve {
                place,
                resolve: true,
                ..
            } => format!("resolve the thread on {place}"),
            Self::Resolve {
                place,
                resolve: false,
                ..
            } => format!("open the thread on {place} again"),
            Self::Approve { approve: true, .. } => format!("approve !{iid}"),
            Self::Approve { approve: false, .. } => format!("take back your approval of !{iid}"),
        }
    }

    /// The text it posts, if it posts any.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Note { text, .. } | Self::Line { text, .. } | Self::Reply { text, .. } => {
                Some(text)
            }
            _ => None,
        }
    }

    /// The complete frozen comment destination, including diff references.
    pub(crate) fn target(&self) -> Option<Target> {
        Some(match self {
            Self::Note { .. } => Target::Note,
            Self::Line { at, .. } => Target::Line(at.clone()),
            Self::Reply {
                discussion, place, ..
            } => Target::Reply {
                discussion: discussion.clone(),
                place: place.clone(),
            },
            _ => return None,
        })
    }

    /// The request that makes it.
    pub fn request(&self) -> Result<Req, String> {
        let (project, iid) = self.mr();
        let base = format!(
            "/api/v4/projects/{}/merge_requests/{iid}",
            project.replace('/', "%2F")
        );
        let text_ok = |t: &str| {
            if t.trim().is_empty() {
                Err("an empty comment is not sent".to_string())
            } else {
                Ok(())
            }
        };
        let id_ok = |d: &str| {
            if !d.is_empty() && d.chars().all(|c| c.is_ascii_hexdigit()) {
                Ok(())
            } else {
                Err(format!("\"{d}\" is not a thread's id"))
            }
        };
        Ok(match self {
            Self::Note { text, .. } => {
                text_ok(text)?;
                Req::new(
                    Method::Post,
                    format!("{base}/notes"),
                    Some(json!({ "body": text })),
                )
            }
            Self::Line { at, text, .. } => {
                text_ok(text)?;
                if at.new.is_none() && at.old.is_none() {
                    return Err("a comment needs a line".into());
                }
                let mut position = json!({
                    "position_type": "text",
                    "base_sha": at.refs.base,
                    "start_sha": at.refs.start,
                    "head_sha": at.refs.head,
                    "old_path": at.old_path,
                    "new_path": at.new_path,
                });
                if let Some(n) = at.new {
                    position["new_line"] = json!(n);
                }
                if let Some(o) = at.old {
                    position["old_line"] = json!(o);
                }
                Req::new(
                    Method::Post,
                    format!("{base}/discussions"),
                    Some(json!({ "body": text, "position": position })),
                )
            }
            Self::Reply {
                discussion, text, ..
            } => {
                text_ok(text)?;
                id_ok(discussion)?;
                Req::new(
                    Method::Post,
                    format!("{base}/discussions/{discussion}/notes"),
                    Some(json!({ "body": text })),
                )
            }
            Self::Resolve {
                discussion,
                resolve,
                ..
            } => {
                id_ok(discussion)?;
                Req::new(
                    Method::Put,
                    format!("{base}/discussions/{discussion}"),
                    Some(json!({ "resolved": resolve })),
                )
            }
            Self::Approve {
                approve: true, sha, ..
            } => Req::new(
                Method::Post,
                format!("{base}/approve"),
                Some(match sha {
                    Some(sha) => json!({ "sha": sha }),
                    None => json!({}),
                }),
            ),
            Self::Approve { approve: false, .. } => {
                Req::new(Method::Post, format!("{base}/unapprove"), Some(json!({})))
            }
        })
    }

    /// What is said once GitLab took it.
    fn done(&self) -> String {
        let (_, iid) = self.mr();
        match self {
            Self::Note { .. } => format!("comment posted on !{iid}"),
            Self::Line { at, .. } => format!("comment posted on {}", at.words()),
            Self::Reply { place, .. } => format!("reply posted in the thread on {place}"),
            Self::Resolve {
                place,
                resolve: true,
                ..
            } => format!("the thread on {place} is resolved"),
            Self::Resolve {
                place,
                resolve: false,
                ..
            } => format!("the thread on {place} is open again"),
            Self::Approve { approve: true, .. } => format!("!{iid} approved"),
            Self::Approve { approve: false, .. } => {
                format!("your approval of !{iid} is taken back")
            }
        }
    }
}

/// GitLab's answer to a write, in words.
pub(crate) fn read_answer(status: u16, body: &Value) -> Result<(), String> {
    let said = match &body["message"] {
        Value::String(s) => s.clone(),
        Value::Null => body["error"].as_str().unwrap_or_default().to_string(),
        other => other.to_string(),
    };
    let with = |s: String| {
        if said.is_empty() {
            s
        } else {
            format!("{s}: {said}")
        }
    };
    match status {
        200..=299 => Ok(()),
        401 => Err("GitLab rejected the token (401)".into()),
        403 => Err(with(
            "GitLab says this token may not do that here (403)".into(),
        )),
        404 => Err(with(
            "GitLab does not show this merge request or thread to this token (404)".into(),
        )),
        409 => Err(with(
            "GitLab says the merge request changed meanwhile (409)".into(),
        )),
        _ => Err(with(format!("GitLab answered {status}"))),
    }
}

/// Send `change` once over `client`; what GitLab said.
pub(crate) async fn send_with(client: &Client, change: &Change) -> Result<String, String> {
    let req = change.request()?;
    let (status, body) = client.write(req).await.map_err(|e| format!("{e:#}"))?;
    read_answer(status, &body).map(|_| change.done())
}

// --- typing ---------------------------------------------------------------------

/// What a comment being typed is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Note,
    Line(LineAt),
    Reply { discussion: String, place: String },
}

/// A comment being typed, in the kit's composer.
#[derive(Debug)]
pub(crate) struct Compose {
    pub project: String,
    pub iid: u64,
    pub target: Target,
    pub composer: Composer,
    pub connection: crate::write_context::ConnectionStamp,
    pub status: Status,
    pub source: String,
}

impl Compose {
    pub(crate) fn new(
        project: &str,
        iid: u64,
        target: Target,
        text: &str,
        connection: crate::write_context::ConnectionStamp,
        source: String,
        status: Status,
    ) -> Self {
        Self {
            project: project.into(),
            iid,
            target,
            composer: Composer::new(text),
            connection,
            status,
            source,
        }
    }

    /// Everything typed, the line being edited included.
    pub fn text(&self) -> String {
        self.composer.text()
    }

    /// `✎ comment on StatsController.java line 21`.
    pub fn title(&self) -> String {
        match &self.target {
            Target::Note => format!("✎ comment on !{}", self.iid),
            Target::Line(at) => format!("✎ comment on {}", at.words()),
            Target::Reply { place, .. } => format!("✎ reply in the thread on {place}"),
        }
    }

    pub(crate) fn change(&self) -> Change {
        let (project, iid, text) = (
            self.project.clone(),
            self.iid,
            self.text().trim_end().to_string(),
        );
        match &self.target {
            Target::Note => Change::Note { project, iid, text },
            Target::Line(at) => Change::Line {
                project,
                iid,
                at: at.clone(),
                text,
            },
            Target::Reply { discussion, place } => Change::Reply {
                project,
                iid,
                discussion: discussion.clone(),
                place: place.clone(),
                text,
            },
        }
    }
}

/// Start only with an idle editor. Existing occurrences are offered, never replaced.
pub(crate) fn start(p: &mut GitlabPlugin, project: &str, iid: u64, target: Target) -> Vec<Effect> {
    if comment_recovery::editor_busy(p) {
        return vec![Effect::Notify(
            "finish the current editor before starting another comment".into(),
        )];
    }
    let destination = Destination {
        project: project.into(),
        iid,
        target: target.clone(),
        connection: p.connection_stamp(),
    };
    if comment_recovery::has_target(p, &destination) {
        return comment_recovery::list(p, Some(destination));
    }
    p.change = None;
    p.review = None;
    p.recovery.clear_choice();
    p.compose = Some(Compose::new(
        project,
        iid,
        target,
        "",
        p.connection_stamp(),
        comment_recovery::source(p),
        Status::Unsent,
    ));
    vec![]
}

/// Keys while a comment is typed.
pub(crate) fn compose_key(p: &mut GitlabPlugin, key: Chord) -> Outcome {
    let Some(c) = p.compose.as_mut() else {
        return Outcome::Ignored;
    };
    match c.composer.key(key) {
        Typed::Review if c.composer.is_empty() => Outcome::Handled(vec![Effect::Notify(
            "nothing to post yet: type the comment".into(),
        )]),
        Typed::Review => {
            let change = c.change();
            Outcome::Handled(ask(p, change))
        }
        Typed::Stop => {
            p.compose = None;
            p.change = None;
            p.review = None;
            Outcome::done()
        }
        Typed::AskStop => Outcome::Handled(vec![Effect::Menu(composer::stop_menu(
            DROP_MENU, "comment",
        ))]),
        Typed::Typing => Outcome::done(),
    }
}

pub(crate) fn compose_paste(p: &mut GitlabPlugin, text: &str) {
    if let Some(c) = p.compose.as_mut() {
        c.composer.paste(text);
    }
}

/// The status line's keys while typing.
pub(crate) fn hints(p: &GitlabPlugin) -> Option<Vec<KeyHint<'static>>> {
    let mut hints = p.compose.as_ref()?.composer.hints();
    hints.push(KeyHint::new("ctrl-r", "kept comments"));
    Some(hints)
}

// --- asking and sending --------------------------------------------------------------

/// The question before `change` is sent, showing what will be sent: the
/// kit's write confirm (the whole text) or its plain one.
pub(crate) fn ask(p: &mut GitlabPlugin, change: Change) -> Vec<Effect> {
    let review = Review::capture(p, &change);
    if p.compose.as_ref().is_some_and(|c| c.change() != change) {
        return vec![Effect::Notify(
            "finish the current comment before reviewing another change".into(),
        )];
    }
    p.recovery.clear_choice();
    let iid = change.mr().1;
    let menu = match &change {
        Change::Note { text, .. } => Menu::write_confirm(
            CONFIRM_MENU,
            format!("post this on !{iid}?"),
            text,
            "post it",
            "drop it",
        ),
        Change::Line { at, text, .. } => Menu::write_confirm(
            CONFIRM_MENU,
            format!("post this on {}?", at.words()),
            text,
            "post it",
            "drop it",
        ),
        Change::Reply { place, text, .. } => Menu::write_confirm(
            CONFIRM_MENU,
            format!("post this in the thread on {place}?"),
            text,
            "post it",
            "drop it",
        ),
        Change::Resolve { place, resolve, .. } => Menu::write_plain(
            CONFIRM_MENU,
            if *resolve {
                format!("resolve the thread on {place}?")
            } else {
                format!("open the thread on {place} again?")
            },
            vec![if *resolve {
                "marks it resolved for everyone".into()
            } else {
                "marks it unresolved for everyone".into()
            }],
            if *resolve {
                "resolve it"
            } else {
                "open it again"
            },
            "leave it as it is",
        ),
        Change::Approve { approve, sha, .. } => {
            let me = if p.me.is_empty() {
                "you".to_string()
            } else {
                p.me.clone()
            };
            let note = match (approve, sha) {
                (true, Some(sha)) => format!("as {me}, of commit {}", &sha[..sha.len().min(8)]),
                (true, None) => format!("as {me}"),
                (false, _) => format!("{me}'s approval comes off"),
            };
            Menu::write_plain(
                CONFIRM_MENU,
                if *approve {
                    format!("approve !{iid}?")
                } else {
                    format!("take back your approval of !{iid}?")
                },
                vec![note],
                if *approve {
                    "approve it"
                } else {
                    "take it back"
                },
                "leave it as it is",
            )
        }
    };
    let menu = match change.kind() {
        writes::Change::Add => menu.adds(),
        writes::Change::Edit => menu.edits(),
    };
    let menu = if review.status == Status::Uncertain {
        menu.saying(Status::Uncertain.words())
    } else {
        menu
    };
    let journal = change.journal(p);
    p.review = Some(review);
    p.change = Some(change);
    vec![journal, Effect::Menu(menu)]
}

/// A write finished.
pub(crate) struct WroteMsg {
    pub operation: u64,
    pub connection: crate::write_context::ConnectionStamp,
    pub change: Change,
    pub result: Result<String, String>,
}

/// Capacity is reserved before consuming the reviewed change or the editor.
fn send(p: &mut GitlabPlugin) -> Vec<Effect> {
    let (Some(change), Some(review)) = (p.change.as_ref(), p.review.as_ref()) else {
        return vec![];
    };
    let mut fx = vec![Effect::Report(JournalOutcome::Confirmed)];
    let connection = review.connection;
    let key = change.key();
    if p.acting
        .get(&key)
        .is_some_and(|a| a.state == Acting::Running)
    {
        fx.extend([
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify("GitLab has not answered the last change yet".into()),
        ]);
        return fx;
    }
    if !review.matches(p, change) || !p.is_current_connection(connection) {
        fx.extend([
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify(
                "this comment or connection changed; your text is still here · ctrl-r to copy it"
                    .into(),
            ),
        ]);
        return fx;
    }
    let refused = writes::not_sent(change.kind());
    let operation = match p.recovery.reserve(change, review, refused.is_none()) {
        Ok(id) => id,
        Err(()) => {
            fx.push(Effect::Report(JournalOutcome::Blocked));
            fx.extend(comment_recovery::full());
            return fx;
        }
    };
    let change = p.change.take().expect("reviewed change");
    p.review = None;
    p.compose = None;
    if let Some(why) = refused {
        fx.extend(finish(
            p,
            WroteMsg {
                operation,
                connection,
                change,
                result: Err(why),
            },
            JournalOutcome::Blocked,
        ));
        return fx;
    }
    p.acting.insert(
        key,
        Act {
            what: change.what(),
            state: Acting::Running,
            at: p.now,
        },
    );
    log::info(format!("gitlab {} on {}", change.what(), change.key()));
    let provider = p.provider();
    fx.extend([
        Effect::Report(JournalOutcome::Started),
        Effect::Notify(SERVICE.asking(&change.doing())),
        Effect::Spawn(Box::pin(async move {
            let result = async {
                let client = provider.client().map_err(|e| format!("{e:#}"))?;
                send_with(&client, &change).await
            }
            .await;
            Box::new(WroteMsg {
                operation,
                connection,
                change,
                result,
            }) as Msg
        })),
    ]);
    fx
}

/// Record what GitLab said and say it; text it refused is kept for `c`.
pub(crate) fn on_wrote(p: &mut GitlabPlugin, msg: WroteMsg) -> Vec<Effect> {
    if !p.recovery.owns(&msg) {
        return completion_skipped(&msg);
    }
    p.recovery.finish(&msg);
    // An error string does not tell us whether the remote mutation executed.
    let outcome = if msg.result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Unknown
    };
    finish(p, msg, outcome)
}

pub(crate) fn completion_skipped(msg: &WroteMsg) -> Vec<Effect> {
    // Actual returned evidence keeps its original task correlation, but cannot
    // consume another reservation, replace action state or start follow-up reads.
    vec![
        Effect::Report(if msg.result.is_ok() {
            JournalOutcome::Succeeded
        } else {
            JournalOutcome::Unknown
        }),
        comment_recovery::observed("gitlab.write.completion.skipped", JournalOutcome::Blocked),
    ]
}

fn finish(p: &mut GitlabPlugin, msg: WroteMsg, outcome: JournalOutcome) -> Vec<Effect> {
    let WroteMsg {
        operation,
        change,
        result,
        ..
    } = msg;
    let (state, said) = match result {
        Ok(m) => (Acting::Done(m.clone()), writes::done(&m)),
        Err(e) => {
            let words = failure_words(&change.doing(), &e, outcome);
            let mut said = words.clone();
            if p.recovery.records.contains_key(&operation) {
                said.push_str(&writes::kept("c"));
            }
            log::warn(format!(
                "gitlab {} on {}: {words}",
                change.what(),
                change.key()
            ));
            (Acting::Failed(words), writes::failed(&said))
        }
    };
    p.acting.insert(
        change.key(),
        Act {
            what: change.what(),
            state,
            at: p.now,
        },
    );
    vec![Effect::Report(outcome), said]
}

/// A choice in one of these menus; `None` when the menu is not theirs.
pub(crate) fn on_menu(p: &mut GitlabPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    Some(match menu {
        CONFIRM_MENU => match value {
            "yes" => send(p),
            "edit" => {
                p.change = None;
                p.review = None;
                vec![Effect::Report(JournalOutcome::Cancelled)]
            }
            _ => {
                let (Some(change), Some(review)) = (p.change.as_ref(), p.review.as_ref()) else {
                    return Some(vec![]);
                };
                if !review.matches(p, change) {
                    return Some(vec![Effect::Notify(
                        "the comment changed; your text is still here".into(),
                    )]);
                }
                if p.recovery.reserve(change, review, false).is_err() {
                    let mut fx = vec![Effect::Report(JournalOutcome::Cancelled)];
                    fx.extend(comment_recovery::full());
                    return Some(fx);
                }
                let kept = change.text().is_some();
                p.compose = None;
                p.change = None;
                p.review = None;
                let mut said = "nothing sent".to_string();
                if kept {
                    said.push_str(&writes::kept("c"));
                }
                vec![
                    Effect::Report(JournalOutcome::Cancelled),
                    Effect::Notify(said),
                ]
            }
        },
        DROP_MENU => {
            if value == "yes" {
                p.compose = None;
                p.change = None;
                p.review = None;
                vec![Effect::Notify("dropped; nothing sent".into())]
            } else {
                vec![]
            }
        }
        _ => return None,
    })
}

pub(crate) fn overlay(p: &GitlabPlugin) -> Option<radar_core::view::PaneView> {
    let c = p.compose.as_ref()?;
    let title = c.title();
    let editor = radar_core::view::Editor {
        title: title.clone(),
        lines: c.composer.lines.clone(),
        line: c.composer.input.value(),
        caret: c.composer.input.cursor(),
        selection: c.composer.input.selection(),
    };
    Some(radar_core::view::PaneView::new(
        radar_core::view::PaneId::Middle,
        title,
        editor.node(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::recorded::{Replay, block};

    fn at() -> LineAt {
        LineAt {
            new_path: "src/a.rs".into(),
            old_path: "src/a.rs".into(),
            old: Some(12),
            new: Some(14),
            refs: DiffRefs {
                base: "b".into(),
                head: "h".into(),
                start: "s".into(),
            },
        }
    }

    fn mr() -> (String, u64) {
        ("acme/team-a/api".into(), 263)
    }

    #[test]
    fn journal_records_scoped_intent_and_explicit_outcomes_without_authored_text() {
        let mut p = crate::tests::sample();
        p.cfg.url = "https://user:password@git.example/?token=secret#secret".into();
        let change = Change::Reply {
            project: "acme/api".into(),
            iid: 12,
            discussion: "ab12".into(),
            place: "secret file name".into(),
            text: "private comment and token".into(),
        };
        let fx = ask(&mut p, change.clone());
        let Effect::Journal(action) = &fx[0] else {
            panic!("missing journal intent")
        };
        assert_eq!(action.kind.as_str(), "gitlab.reply.post");
        assert_eq!(action.subjects[0].key.instance, "https://git.example/");
        assert_eq!(action.subjects[0].key.id, "acme/api!12");
        let stored = format!("{action:?}");
        for private in ["password", "secret", "private comment", "token"] {
            assert!(!stored.contains(private), "journal contains {private}");
        }
        assert!(
            on_menu(&mut p, CONFIRM_MENU, "no")
                .unwrap()
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Cancelled)))
        );
        assert!(
            on_wrote(
                &mut p,
                WroteMsg {
                    operation: 0,
                    connection: Default::default(),
                    change: change.clone(),
                    result: Ok("posted".into())
                }
            )
            .iter()
            .any(|e| matches!(e, Effect::Report(JournalOutcome::Succeeded)))
        );
        let fx = on_wrote(
            &mut p,
            WroteMsg {
                operation: 0,
                connection: Default::default(),
                change: change.clone(),
                result: Err("raw secret response".into()),
            },
        );
        assert!(matches!(fx[0], Effect::Report(JournalOutcome::Unknown)));
        let old = writes::mode();
        writes::set_mode(writes::Mode::ReadOnly, None);
        ask(&mut p, change);
        let blocked = send(&mut p);
        writes::set_mode(old.0, old.1);
        assert!(
            blocked
                .iter()
                .any(|e| matches!(e, Effect::Report(JournalOutcome::Blocked)))
        );
        assert!(!blocked.iter().any(|e| matches!(e, Effect::Spawn(_))));
    }

    #[test]
    fn each_write_is_the_request_gitlab_documents() {
        let (project, iid) = mr();
        let base = "/api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/263";
        let r = Change::Note {
            project: project.clone(),
            iid,
            text: "Looks right.".into(),
        }
        .request()
        .unwrap();
        assert_eq!(
            (r.method, r.path.as_str()),
            (Method::Post, format!("{base}/notes").as_str())
        );
        assert_eq!(r.body, Some(json!({ "body": "Looks right." })));

        let r = Change::Line {
            project: project.clone(),
            iid,
            at: at(),
            text: "Cap it?".into(),
        }
        .request()
        .unwrap();
        assert_eq!(r.path, format!("{base}/discussions"));
        assert_eq!(
            r.body.unwrap()["position"],
            json!({ "position_type": "text", "base_sha": "b", "start_sha": "s", "head_sha": "h",
                    "old_path": "src/a.rs", "new_path": "src/a.rs", "old_line": 12, "new_line": 14 })
        );
        // An added line names its new line only.
        let mut added = at();
        added.old = None;
        let r = Change::Line {
            project: project.clone(),
            iid,
            at: added,
            text: "x".into(),
        }
        .request()
        .unwrap();
        assert!(r.body.unwrap()["position"].get("old_line").is_none());

        let r = Change::Reply {
            project: project.clone(),
            iid,
            discussion: "6a9c1750".into(),
            place: "a.rs:14".into(),
            text: "Done.".into(),
        }
        .request()
        .unwrap();
        assert_eq!(r.path, format!("{base}/discussions/6a9c1750/notes"));

        let r = Change::Resolve {
            project: project.clone(),
            iid,
            discussion: "6a9c1750".into(),
            place: "a.rs:14".into(),
            resolve: false,
        }
        .request()
        .unwrap();
        assert_eq!(
            (r.method, r.body),
            (Method::Put, Some(json!({ "resolved": false })))
        );

        let r = Change::Approve {
            project: project.clone(),
            iid,
            approve: true,
            sha: Some("abc".into()),
        }
        .request()
        .unwrap();
        assert_eq!(
            (r.path.as_str(), r.body),
            (
                format!("{base}/approve").as_str(),
                Some(json!({ "sha": "abc" }))
            )
        );
        let r = Change::Approve {
            project: project.clone(),
            iid,
            approve: false,
            sha: None,
        }
        .request()
        .unwrap();
        assert_eq!(r.path, format!("{base}/unapprove"));

        // Refused before anything is built.
        assert!(
            Change::Note {
                project: project.clone(),
                iid,
                text: " \n".into()
            }
            .request()
            .is_err()
        );
        assert!(
            Change::Reply {
                project: project.clone(),
                iid,
                discussion: "../x".into(),
                place: String::new(),
                text: "x".into()
            }
            .request()
            .is_err()
        );
    }

    #[test]
    fn a_write_is_sent_once_and_gitlabs_refusal_said_as_it_is() {
        let (project, iid) = mr();
        let change = Change::Note {
            project,
            iid,
            text: "Looks right.".into(),
        };
        for status in [429, 500, 502] {
            let r = Replay::new(vec![(status, json!({})), (201, json!({ "id": 1 }))]);
            let got = block(send_with(&Client::over(r.clone()), &change));
            assert!(got.is_err(), "{status}");
            assert_eq!(r.asked().len(), 1, "{status}: sent once");
        }
        let r = Replay::new(vec![(403, json!({ "message": "403 Forbidden" }))]);
        assert_eq!(
            block(send_with(&Client::over(r), &change)),
            Err("GitLab says this token may not do that here (403): 403 Forbidden".into())
        );
        let r = Replay::new(vec![(201, json!({ "id": 1 }))]);
        let asked = r.clone();
        assert_eq!(
            block(send_with(&Client::over(r), &change)),
            Ok("comment posted on !263".into())
        );
        assert_eq!(
            asked.asked()[0].body,
            Some(json!({ "body": "Looks right." }))
        );
    }
}
