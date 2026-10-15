//! Rebase and merge a merge request. GitLab is the authority: radar offers a
//! merge only when the last sync says GitLab would take it, asks before
//! either, checks again with GitLab just before a merge (and merges exactly
//! the commit it checked), and shows whatever GitLab answers.
//!
//! A rebase goes through the REST API (`PUT …/merge_requests/:iid/rebase`):
//! GitLab's GraphQL has no rebase. A merge goes through GraphQL
//! (`mergeRequestAccept`). The requests are built and read by plain
//! functions here, so they are tested without a server.

use radar_core::plugin::{Effect, Msg};
use radar_core::journal::{ActionDescriptor, Origin, Outcome as JournalOutcome, SubjectKey, SubjectRole, SubjectSnapshot};
use serde_json::{Value, json};

use crate::model::{MergeSettings, Mr, MrState};
use crate::{Client, GitlabPlugin, config};

/// What GitLab says about a merge request right before a merge.
pub const FRESH_QUERY: &str = r#"
query($project: ID!, $iid: String!) {
  project(fullPath: $project) {
    mergeRequest(iid: $iid) {
      diffHeadSha detailedMergeStatus draft conflicts approved approvalsLeft state
    }
  }
}"#;

/// Merge, but only the commit that was checked (`sha`), squashing and
/// deleting the source branch as the question showed (null: as GitLab is
/// set).
pub const ACCEPT_MUTATION: &str = r#"
mutation($project: ID!, $iid: String!, $sha: String!, $squash: Boolean, $remove: Boolean) {
  mergeRequestAccept(input: {projectPath: $project, iid: $iid, sha: $sha, squash: $squash, shouldRemoveSourceBranch: $remove}) {
    errors
    mergeRequest { state detailedMergeStatus }
  }
}"#;

/// The REST path that asks GitLab to rebase a merge request's branch.
pub fn rebase_path(project: &str, iid: u64) -> String {
    format!(
        "projects/{}/merge_requests/{iid}/rebase",
        project.replace('/', "%2F")
    )
}

pub fn fresh_vars(project: &str, iid: u64) -> Value {
    json!({ "project": project, "iid": iid.to_string() })
}

pub fn accept_vars(project: &str, iid: u64, sha: &str, how: Option<MergeSettings>) -> Value {
    json!({
        "project": project, "iid": iid.to_string(), "sha": sha,
        "squash": how.map(|h| h.squash), "remove": how.map(|h| h.remove_source),
    })
}

/// Why GitLab would not merge it now, in words; `None` when it would.
fn blocker_of(
    draft: bool,
    conflicts: bool,
    approved: bool,
    approvals_left: u64,
    status: &str,
) -> Option<String> {
    if draft {
        return Some("still a draft".into());
    }
    if conflicts {
        return Some("has conflicts".into());
    }
    if !approved {
        let n = approvals_left.max(1);
        return Some(format!(
            "needs {n} more approval{}",
            if n == 1 { "" } else { "s" }
        ));
    }
    match status {
        "MERGEABLE" => None,
        "" => Some("GitLab has not said whether it can merge".into()),
        "NEED_REBASE" => Some("needs a rebase first".into()),
        "CI_MUST_PASS" => Some("the pipeline must pass first".into()),
        "CI_STILL_RUNNING" => Some("the pipeline is still running".into()),
        "DISCUSSIONS_NOT_RESOLVED" => Some("threads are unresolved".into()),
        "CHECKING" | "UNCHECKED" | "PREPARING" => Some("GitLab is still checking it".into()),
        "BLOCKED_STATUS" => Some("blocked by another merge request".into()),
        other => Some(other.to_ascii_lowercase().replace('_', " ")),
    }
}

/// Why this merge request cannot be merged now, as the last sync saw it.
pub fn blocker(mr: &Mr) -> Option<String> {
    if let Some(over) = over_already(mr) {
        return Some(over);
    }
    // The same words as its STATE.
    crate::ready::first_words(mr)
}

/// `it is merged already`, for a merge request that is not open.
fn over_already(mr: &Mr) -> Option<String> {
    (mr.state != MrState::Opened).then(|| format!("it is {} already", mr.state.word()))
}

/// Why a rebase is not offered: GitLab does not say this merge request needs
/// one. `None` when it does (it is behind its target).
pub fn rebase_blocker(mr: &Mr) -> Option<String> {
    if let Some(over) = over_already(mr) {
        return Some(over);
    }
    let blocks = crate::ready::blocks(mr);
    if blocks.contains(&crate::ready::Block::Rebase) {
        return None;
    }
    Some(match blocks.first() {
        Some(crate::ready::Block::Checking) => "GitLab is still checking it".into(),
        Some(crate::ready::Block::Conflicts) => "it has conflicts, which a rebase cannot fix".into(),
        _ => format!("not behind {}", mr.target_branch),
    })
}

/// GitLab's answer to [`FRESH_QUERY`]: the commit to merge, or why not.
pub fn read_fresh(d: &Value) -> Result<String, String> {
    let m = &d["project"]["mergeRequest"];
    if m.is_null() {
        return Err("GitLab no longer shows this merge request".into());
    }
    let state = m["state"].as_str().unwrap_or_default();
    if state != "opened" {
        return Err(format!("it is {state} already"));
    }
    if let Some(why) = blocker_of(
        m["draft"].as_bool().unwrap_or(false),
        m["conflicts"].as_bool().unwrap_or(false),
        m["approved"].as_bool().unwrap_or(false),
        m["approvalsLeft"].as_u64().unwrap_or(0),
        m["detailedMergeStatus"].as_str().unwrap_or_default(),
    ) {
        return Err(why);
    }
    match m["diffHeadSha"].as_str() {
        Some(sha) if !sha.is_empty() => Ok(sha.to_string()),
        _ => Err("GitLab did not say which commit it would merge".into()),
    }
}

/// GitLab's answer to [`ACCEPT_MUTATION`].
pub fn read_accept(d: &Value) -> Result<String, String> {
    let r = &d["mergeRequestAccept"];
    let errors: Vec<&str> = r["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    match r["mergeRequest"]["state"].as_str() {
        Some("merged") => Ok("merged".into()),
        Some(state) => Ok(format!("GitLab took it; it is {state} for now")),
        None => Err("GitLab did not answer about the merge".into()),
    }
}

/// GitLab's answer to the rebase request.
pub fn read_rebase(status: u16, body: &Value) -> Result<String, String> {
    let said = match &body["message"] {
        Value::String(s) => s.clone(),
        Value::Null => body["error"].as_str().unwrap_or_default().to_string(),
        other => other.to_string(),
    };
    match status {
        200..=299 => Ok("rebase requested: GitLab is rebasing the branch".into()),
        401 => Err("GitLab rejected the token (401)".into()),
        403 if said.is_empty() => Err("GitLab says you may not push to its branch (403)".into()),
        404 => Err("GitLab does not show this merge request to this token (404)".into()),
        _ if said.is_empty() => Err(format!("GitLab answered {status}")),
        _ => Err(format!("GitLab: {said}")),
    }
}

/// Where an action on a merge request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acting {
    Running,
    Done(String),
    /// Complete outcome-aware sentence, shared by the notice and page.
    Failed(String),
}

/// The last rebase or merge asked for on a merge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Act {
    /// `rebase` or `merge`.
    pub what: &'static str,
    pub state: Acting,
    pub at: i64,
}

/// A rebase or merge finished.
pub struct ActMsg {
    pub connection: crate::write_context::ConnectionStamp,
    pub key: String,
    pub what: &'static str,
    pub result: Result<String, String>,
    pub outcome: JournalOutcome,
}

/// Rebase success means GitLab accepted the request, not that its background job ended.
pub(crate) fn journal(p: &GitlabPlugin, operation: &'static str, project: &str, iid: u64) -> Effect {
    let mut action = ActionDescriptor::new(operation, Origin::Plugin("gitlab".into()));
    if let Ok(key) = SubjectKey::new("gitlab", &p.cfg.url, "merge-request", &format!("{project}!{iid}")) {
        action = action.with_subject(SubjectSnapshot::new(key, SubjectRole::Primary));
    } else {
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
    }
    Effect::Journal(action)
}

pub fn running(p: &GitlabPlugin) -> bool {
    p.acting.values().any(|a| a.state == Acting::Running)
}

/// Send the rebase or merge (`what`), already confirmed by the user.
pub fn start(p: &mut GitlabPlugin, project: &str, iid: u64, what: &'static str) -> Vec<Effect> {
    let mut fx = vec![Effect::Report(JournalOutcome::Confirmed)];
    let connection = p.connection_stamp();
    let key = crate::pages::mr_key(project, iid);
    if p.acting.get(&key).is_some_and(|a| a.state == Acting::Running) {
        fx.extend([Effect::Report(JournalOutcome::Blocked), Effect::Notify(format!("already asked GitLab for a {what}"))]);
        return fx;
    }
    let how = p.merge_as.remove(&key).flatten();
    // A rebase and a merge change what is there: edit mode.
    if let Some(why) = radar_core::writes::not_sent(radar_core::writes::Change::Edit) {
        fx.extend(on_msg(
            p,
            ActMsg {
                connection,
                key,
                what,
                result: Err(why),
                outcome: JournalOutcome::Blocked,
            },
        ));
        return fx;
    }
    p.acting.insert(
        key.clone(),
        Act {
            what,
            state: Acting::Running,
            at: p.now,
        },
    );
    let (url, token, project) = (p.cfg.url.clone(), p.cfg.token.clone(), project.to_string());
    let task = Effect::Spawn(Box::pin(async move {
        // Before a mutation starts, a failed prerequisite cannot have changed GitLab.
        let mut outcome = JournalOutcome::Failed;
        let result = async {
            let token = config::resolve_token(&token, &url).map_err(|e| format!("{e:#}"))?;
            let client = Client::new(&url, token);
            if what == "rebase" {
                outcome = JournalOutcome::Unknown;
                let (status, body) = client
                    .put_rest(&rebase_path(&project, iid))
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                let result = read_rebase(status, &body);
                outcome = if result.is_ok() {
                    JournalOutcome::Succeeded
                } else if (400..500).contains(&status) {
                    JournalOutcome::Failed
                } else {
                    JournalOutcome::Unknown
                };
                result
            } else {
                let d = client
                    .graphql(FRESH_QUERY, fresh_vars(&project, iid))
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                outcome = JournalOutcome::Blocked;
                let sha = read_fresh(&d)?;
                outcome = JournalOutcome::Unknown;
                let d = client
                    .graphql(ACCEPT_MUTATION, accept_vars(&project, iid, &sha, how))
                    .await
                    .map_err(|e| format!("{e:#}"))?;
                outcome = merge_outcome(&d);
                read_accept(&d)
            }
        }
        .await;
        Box::new(ActMsg { connection, key, what, result, outcome }) as Msg
    }));
    fx.extend([Effect::Report(JournalOutcome::Started), task, Effect::Notify(crate::write::SERVICE.asking(&format!("{what} !{iid}")))]);
    fx
}

fn merge_outcome(d: &Value) -> JournalOutcome {
    let answer = &d["mergeRequestAccept"];
    if answer["errors"].as_array().is_some_and(|errors| !errors.is_empty()) {
        JournalOutcome::Failed
    } else if answer["mergeRequest"]["state"].as_str() == Some("merged") {
        JournalOutcome::Succeeded
    } else {
        // Accepted or malformed is not proof that the merge completed.
        JournalOutcome::Unknown
    }
}

/// Record how it went and say so; the caller refreshes.
pub fn on_msg(p: &mut GitlabPlugin, msg: ActMsg) -> Vec<Effect> {
    let iid = msg.key.rsplit_once('!').map_or("", |(_, iid)| iid);
    let (state, said) = match msg.result {
        Ok(m) => (Acting::Done(m.clone()), radar_core::writes::done(&m)),
        Err(e) => {
            let words = crate::write::failure_words(
                &format!("{} !{iid}", msg.what),
                &e,
                msg.outcome,
            );
            (Acting::Failed(words.clone()), radar_core::writes::failed(&words))
        }
    };
    p.acting.insert(
        msg.key,
        Act {
            what: msg.what,
            state,
            at: p.now,
        },
    );
    vec![Effect::Report(msg.outcome), said]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::mr;

    #[test]
    fn the_rebase_request_names_the_project_and_reads_gitlabs_answer() {
        assert_eq!(
            rebase_path("acme/team-a/api", 263),
            "projects/acme%2Fteam-a%2Fapi/merge_requests/263/rebase"
        );
        assert!(read_rebase(202, &json!({"rebase_in_progress": true})).is_ok());
        assert_eq!(
            read_rebase(403, &json!({"message": "403 Forbidden"})),
            Err("GitLab: 403 Forbidden".into())
        );
        assert!(read_rebase(403, &Value::Null).unwrap_err().contains("may not push"));
        assert!(read_rebase(409, &json!({"message": {"base": ["Rebase in progress"]}}))
            .unwrap_err()
            .contains("Rebase in progress"));
    }

    #[test]
    fn a_merge_is_blocked_in_words_until_gitlab_would_take_it() {
        let mut m = mr(1, "me", "acme/team-a/api");
        // The same words as the merge request's STATE.
        assert_eq!(blocker(&m).as_deref(), Some("needs 1 approval"));
        m.approvals_left = 2;
        assert_eq!(blocker(&m).as_deref(), Some("needs 2 approvals"));
        m.approved = true;
        m.merge_status = "CI_MUST_PASS".into();
        m.pipeline = Some(crate::model::Pipeline::Failed);
        assert_eq!(blocker(&m).as_deref(), Some("pipeline failed"));
        m.pipeline = Some(crate::model::Pipeline::Passed);
        m.merge_status = "MERGEABLE".into();
        assert_eq!(blocker(&m), None);
        m.draft = true;
        assert_eq!(blocker(&m).as_deref(), Some("draft"));
    }

    #[test]
    fn the_merge_checks_again_and_merges_only_the_commit_it_checked() {
        let fresh = |status: &str, state: &str| {
            json!({"project": {"mergeRequest": {"diffHeadSha": "abc123", "detailedMergeStatus": status,
                "draft": false, "conflicts": false, "approved": true, "approvalsLeft": 0, "state": state}}})
        };
        assert_eq!(read_fresh(&fresh("MERGEABLE", "opened")), Ok("abc123".into()));
        assert_eq!(
            read_fresh(&fresh("NEED_REBASE", "opened")),
            Err("needs a rebase first".into())
        );
        assert_eq!(read_fresh(&fresh("MERGEABLE", "merged")), Err("it is merged already".into()));
        assert!(read_fresh(&json!({"project": {"mergeRequest": null}})).is_err());
        let how = MergeSettings { squash: true, remove_source: false, remove_forced: false };
        let vars = accept_vars("acme/team-a/api", 263, "abc123", Some(how));
        assert_eq!(
            vars,
            json!({"project": "acme/team-a/api", "iid": "263", "sha": "abc123", "squash": true, "remove": false})
        );
        let vars = accept_vars("acme/team-a/api", 263, "abc123", None);
        assert!(vars["squash"].is_null() && vars["remove"].is_null(), "as GitLab is set");
        assert!(ACCEPT_MUTATION.contains("sha: $sha") && ACCEPT_MUTATION.contains("shouldRemoveSourceBranch: $remove"));
    }

    #[test]
    fn gitlabs_answer_to_the_merge_is_shown_as_it_is() {
        let ok = json!({"mergeRequestAccept": {"errors": [], "mergeRequest": {"state": "merged"}}});
        assert_eq!(read_accept(&ok), Ok("merged".into()));
        assert_eq!(merge_outcome(&ok), JournalOutcome::Succeeded);
        let refused = json!({"mergeRequestAccept": {"errors": ["SHA does not match HEAD of source branch"],
            "mergeRequest": {"state": "opened"}}});
        assert_eq!(
            read_accept(&refused),
            Err("SHA does not match HEAD of source branch".into())
        );
        assert_eq!(merge_outcome(&refused), JournalOutcome::Failed);
        assert_eq!(merge_outcome(&json!({"mergeRequestAccept": {"errors": [], "mergeRequest": {"state": "opened"}}})), JournalOutcome::Unknown);
        assert_eq!(merge_outcome(&json!({})), JournalOutcome::Unknown);
        assert!(read_accept(&json!({})).is_err());
    }

    #[test]
    fn an_action_is_recorded_while_it_runs_and_when_it_ends() {
        let mut p = crate::tests::sample();
        let fx = start(&mut p, "acme/team-a/api", 263, "rebase");
        assert!(fx.iter().any(|effect| matches!(effect, Effect::Spawn(_))));
        assert!(running(&p));
        let again = start(&mut p, "acme/team-a/api", 263, "rebase");
        assert!(again.iter().any(|effect| matches!(effect, Effect::Notify(m) if m.starts_with("already"))));
        let connection = p.connection_stamp();
        let fx = on_msg(
            &mut p,
            ActMsg {
                connection,
                key: "acme/team-a/api!263".into(),
                what: "rebase",
                result: Err("GitLab: 403 Forbidden".into()),
                outcome: JournalOutcome::Failed,
            },
        );
        assert!(fx.iter().any(|effect| matches!(effect, Effect::Notify(m) if m == "✗ GitLab did not rebase !263: GitLab: 403 Forbidden")), "{fx:?}");
        assert!(!running(&p));
    }
}
