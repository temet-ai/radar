//! What a merge request tells the host so an agent can be briefed on it
//! (send to agent), and its resolution for the review journey (project,
//! branches, head sha, diff stats, description, open threads).
//!
//! Built from what the plugin already holds (a list row, and the page's
//! detail when it was opened); the resolution is fetched with the page's own
//! detail query ([`crate::client::Client::mr_detail`]), whose complexity is
//! the one the merge request page already pays.

use radar_core::agent_context::{
    AgentContext, AgentContextRequest, ChangeResolution, ChangeResolveRequest, ContextField,
    DiffStats, REVIEW_LOCALLY_ACTION, Resolve, ReviewRequest, SEND_TO_AGENT_ACTION, ThreadSummary,
};
use radar_core::journal::{SafeUrl, SubjectKey};
use radar_core::plugin::{Binding, Effect, Outcome, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::ui::icon::Icon;

use crate::{GitlabPlugin, Row, act, head};
use crate::join::MERGE_REQUEST;
use crate::model::{Discussion, Mr, MrDetail};
use crate::pages::mr_key;

/// The word the brief heads the quoted material with.
pub(crate) const KIND: &str = "merge request";
/// Open threads a context or a resolution carries; the rest are counted.
pub(crate) const MAX_THREADS: usize = 20;
/// Bytes of one thread's first comment in a context.
const THREAD_BODY_BYTES: usize = 600;
/// Files a context lists by path.
pub(crate) const MAX_FILES: usize = 40;

/// The scoped identity of a merge request on this server.
pub(crate) fn key(base_url: &str, project: &str, iid: u64) -> Option<SubjectKey> {
    SubjectKey::new("gitlab", base_url, MERGE_REQUEST, &format!("{project}!{iid}")).ok()
}

/// A merge request's subject id (`group/project!12`) split into project and iid.
pub(crate) fn split_id(id: &str) -> Option<(&str, u64)> {
    let (project, iid) = id.rsplit_once('!')?;
    Some((project, iid.parse().ok()?))
}

/// Open (unresolved, resolvable) discussions, oldest first, the first note of each.
fn open_threads(detail: &MrDetail) -> impl Iterator<Item = &Discussion> {
    detail
        .discussions
        .iter()
        .filter(|d| d.resolvable && !d.resolved && !d.notes.is_empty())
}

fn thread_summary(d: &Discussion) -> ThreadSummary {
    let n = &d.notes[0];
    ThreadSummary {
        author: n.author.clone(),
        file: n.path.clone(),
        line: n.line,
        body: n.body.clone(),
    }
}

/// The context for a merge request: its row, plus the page's detail when held.
/// Never fetches.
pub(crate) fn context(base_url: &str, mr: &Mr, detail: Option<&MrDetail>) -> Option<AgentContext> {
    let key = key(base_url, &mr.project_path, mr.iid)?;
    let mut c = AgentContext::new(KIND, key, &format!("!{} {}", mr.iid, mr.title))
        .link(Some(mr.web_url.clone()))
        .field(ContextField::new("project", &mr.project_path))
        .field(ContextField::new("source_branch", &mr.source_branch))
        .field(ContextField::new("target_branch", &mr.target_branch))
        .field(ContextField::new("state", mr.state.word()))
        .field(ContextField::new("draft", if mr.draft { "yes" } else { "no" }))
        .field(ContextField::new("author", &mr.author.username))
        .field(ContextField::new("merge_status", &mr.merge_status))
        .field(ContextField::new(
            "diff_stats",
            format!("{} files, +{} -{}", mr.files, mr.additions, mr.deletions),
        ));
    if let Some(p) = mr.pipeline {
        c = c.field(ContextField::new("pipeline", p.word()));
    }
    if !mr.labels.is_empty() {
        c = c.field(ContextField::new("labels", mr.labels.join(", ")));
    }
    let Some(d) = detail else {
        return Some(c);
    };
    if let Some(refs) = &d.refs
        && !refs.head.is_empty()
    {
        c = c.field(ContextField::new("head_sha", &refs.head));
    }
    if !d.description.is_empty() {
        c = c.field(ContextField::new("description", &d.description));
    }
    if !d.files.is_empty() {
        let mut files: Vec<String> = d
            .files
            .iter()
            .take(MAX_FILES)
            .map(|f| format!("{} (+{} -{})", f.path, f.additions, f.deletions))
            .collect();
        if d.files.len() > MAX_FILES {
            files.push(format!("… and {} more", d.files.len() - MAX_FILES));
        }
        c = c.field(ContextField::new("files", files.join("\n")));
    }
    let open: Vec<&Discussion> = open_threads(d).collect();
    if !open.is_empty() {
        // One field: the commenters' words, each thread cut short.
        let lines: Vec<String> = open
            .iter()
            .take(MAX_THREADS)
            .map(|t| {
                let n = &t.notes[0];
                let place = t.place().map(|p| format!(" at {p}")).unwrap_or_default();
                let mut body = n.body.replace('\n', " ");
                radar_core::agent_context::cap_text(&mut body, THREAD_BODY_BYTES);
                format!("{}{place}: {body}", n.author)
            })
            .collect();
        c = c.field(ContextField::new("open_threads", lines.join("\n")));
    }
    if open.len() > MAX_THREADS {
        c = c.field(ContextField::new(
            "more_open_threads",
            (open.len() - MAX_THREADS).to_string(),
        ));
    }
    Some(c)
}

/// What the review journey needs of a merge request. `None` while GitLab has
/// not given the head sha (the detail was never fetched, or has no diff refs):
/// the worktree is made at exactly this sha, so a guess is worse than no answer.
pub(crate) fn resolution(base_url: &str, mr: &Mr, detail: &MrDetail) -> Option<ChangeResolution> {
    let head = detail.refs.as_ref().map(|r| r.head.clone()).filter(|h| !h.is_empty())?;
    Some(ChangeResolution {
        subject: key(base_url, &mr.project_path, mr.iid)?,
        title: mr.title.clone(),
        link: Some(mr.web_url.clone()),
        repository: mr.project_path.clone(),
        source_branch: mr.source_branch.clone(),
        target_branch: mr.target_branch.clone(),
        head_sha: head,
        diff_stats: DiffStats {
            files: detail.files.len().max(mr.files as usize) as u32,
            additions: mr.additions,
            deletions: mr.deletions,
        },
        description: detail.description.clone(),
        threads: open_threads(detail).take(MAX_THREADS).map(thread_summary).collect(),
    })
}

/// The merge request the agent action acts on: the one selected in the tree
/// (middle or detail pane), or the one whose page is open. None on the nav
/// pane, a repo, a job or when nothing is selected.
fn target(p: &GitlabPlugin, pane: Pane) -> Option<(String, u64)> {
    if pane == Pane::Nav {
        return None;
    }
    match p.pages.last() {
        Some(page) => match &page.kind {
            crate::pages::Kind::Mr { project, iid } => Some((project.clone(), *iid)),
            _ => None,
        },
        None => {
            let items = p.visible();
            let it = crate::screen::selected_mr(p, &items)?;
            Some((it.mr.project_path.clone(), it.mr.iid))
        }
    }
}

/// The key of the agent action on a row (`A`), the same on every part.
pub(crate) const SEND_KEY: char = 'A';

/// The key of "review locally" on a merge request (`rev-i-ew`). Slack's `v`
/// is taken on a merge request: Jira's "open the ticket" and the diff's
/// side-by-side use it.
pub(crate) const REVIEW_KEY: char = 'i';

/// Whether a local review can be asked of `project!iid`: it is held here and
/// still open (a merged or closed one has nothing left to review).
fn reviewable(p: &GitlabPlugin, project: &str, iid: u64) -> bool {
    crate::pages::find_mr(p, project, iid).is_some_and(|mr| mr.state == crate::model::MrState::Opened)
}

/// The `x` rows for the agent actions: the shared ids, written out here.
pub(crate) fn rows(p: &GitlabPlugin, pane: Pane) -> Vec<Row> {
    let Some((project, iid)) = target(p, pane) else {
        return vec![];
    };
    let mut out = vec![
        head(Binding::heading("with an agent").icon(Some(Icon::Bot))),
        act(SEND_TO_AGENT_ACTION, Binding::new(SEND_KEY.to_string(), "ask an agent about it…")),
    ];
    if reviewable(p, &project, iid) {
        out.push(act(REVIEW_LOCALLY_ACTION, Binding::new(REVIEW_KEY.to_string(), "review it locally…")));
    }
    out
}

/// `A` asks the host to send the merge request to an agent; `i` asks it to
/// start the local review of the merge request. Nothing is sent or posted
/// from here: the host asks [`agent_context`] for the context, or the sessions
/// part runs the journey, which asks [`resolve`] and makes a worktree.
pub(crate) fn key_press(p: &GitlabPlugin, pane: Pane, pressed: KeyEvent) -> Option<Outcome> {
    if pressed.modifiers.difference(KeyModifiers::SHIFT) != KeyModifiers::NONE {
        return None;
    }
    let KeyCode::Char(c) = pressed.code else { return None };
    if c != SEND_KEY && c != REVIEW_KEY {
        return None;
    }
    let (project, iid) = target(p, pane)?;
    let base = SafeUrl::parse(&p.cfg.url).ok()?;
    let subject = key(base.as_str(), &project, iid)?;
    let request = AgentContextRequest { subject: subject.clone() };
    Some(Outcome::Handled(if c == SEND_KEY {
        if agent_context(p, &request).is_some() {
            vec![Effect::SendToAgent(request)]
        } else {
            vec![Effect::Notify(format!("!{iid} is not held here: nothing to send"))]
        }
    } else if !reviewable(p, &project, iid) {
        vec![Effect::Notify(format!("!{iid} is not open: nothing to review"))]
    } else {
        // The merge request is its own change request: no link to map, and
        // no origin row to quote beside it (the host leaves it out).
        vec![Effect::StartReview(ReviewRequest { origin: subject.clone(), links: vec![], changes: vec![subject] })]
    }))
}

/// Asked of the plugin: whose server a key is on, so a key from another
/// instance (or provider) is never answered.
fn owned_mr(p: &GitlabPlugin, subject: &SubjectKey) -> Option<(String, u64)> {
    if subject.provider != "gitlab" || subject.kind != MERGE_REQUEST {
        return None;
    }
    let (project, iid) = split_id(&subject.id)?;
    let mine = SafeUrl::parse(&p.cfg.url).ok()?;
    (key(mine.as_str(), project, iid)?.instance == subject.instance).then(|| (project.to_owned(), iid))
}

/// The context for `request`'s merge request, from the list row and the
/// page's detail when held. Never fetches.
pub(crate) fn agent_context(p: &GitlabPlugin, request: &AgentContextRequest) -> Option<AgentContext> {
    let (project, iid) = owned_mr(p, &request.subject)?;
    let held = p.mr_pages.get(&mr_key(&project, iid)).and_then(|f| f.value.as_ref());
    let mr = crate::pages::find_mr(p, &project, iid)?;
    let base = SafeUrl::parse(&p.cfg.url).ok()?;
    context(base.as_str(), mr, held.map(|(_, d)| d))
}

/// A held detail this recent is the head a review is made of.
const FRESH_SECS: i64 = 30;

/// Resolve `request`'s merge request for a review. A detail fetched in the
/// last [`FRESH_SECS`] answers; otherwise the page's own fetch is started
/// (one request, journaled as the page's read is) and the answer is `Pending`.
pub(crate) fn resolve(p: &mut GitlabPlugin, request: &ChangeResolveRequest) -> Option<(Resolve, Vec<Effect>)> {
    let (project, iid) = owned_mr(p, &request.subject)?;
    let key = mr_key(&project, iid);
    let base = SafeUrl::parse(&p.cfg.url).ok()?;
    let now = p.now;
    if let Some(f) = p.mr_pages.get(&key) {
        if f.loading {
            return Some((Resolve::Pending, vec![]));
        }
        if p.resolving.contains(&key) {
            // The fetch this review started has come back.
            if let Some(error) = &f.error {
                p.resolving.remove(&key);
                return Some((Resolve::Failed(format!("GitLab: {error}")), vec![]));
            }
        }
        if let Some((mr, detail)) = &f.value
            && now - f.at < FRESH_SECS
        {
            p.resolving.remove(&key);
            return Some((
                match resolution(base.as_str(), mr, detail) {
                    Some(r) => Resolve::Ready(r),
                    None => Resolve::Failed(format!("GitLab gave no head commit for {project}!{iid}")),
                },
                vec![],
            ));
        }
    }
    p.resolving.insert(key);
    let fx = crate::pages::fetch(p, &crate::pages::Kind::Mr { project, iid }, true);
    Some((Resolve::Pending, fx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures;
    use crate::model::{DiffRefs, MrDetail};
    use radar_core::plugin::Effect;

    const BASE: &str = "https://gitlab.example.test";

    fn detail() -> MrDetail {
        let mut d = fixtures::detail();
        d.refs = Some(DiffRefs { base: "b".into(), head: "abc123".into(), start: "s".into() });
        d
    }

    const PROJECT: &str = "acme/team-a/api";

    fn request(p: &GitlabPlugin, iid: u64) -> SubjectKey {
        let base = SafeUrl::parse(&p.cfg.url).unwrap();
        key(base.as_str(), PROJECT, iid).unwrap()
    }

    fn open_page(p: &mut GitlabPlugin) {
        crate::pages::push(p, crate::pages::Kind::Mr { project: PROJECT.into(), iid: 263 });
    }

    #[test]
    fn a_held_page_resolves_to_its_head_and_branches() {
        let mut p = crate::demo::sample_reviewed(false);
        let subject = request(&p, 263);
        let (r, fx) = resolve(&mut p, &ChangeResolveRequest { subject }).unwrap();
        assert!(fx.is_empty());
        let Resolve::Ready(r) = r else { panic!("{r:?}") };
        assert_eq!(r.repository, PROJECT);
        assert_eq!(r.head_sha, fixtures::detail().refs.unwrap().head);
        assert!(!r.source_branch.is_empty() && !r.target_branch.is_empty());
        assert!(!r.description.is_empty());
        assert!(p.resolving.is_empty());
    }

    #[test]
    fn an_old_page_is_fetched_again_once_and_a_failure_is_told() {
        let mut p = crate::demo::sample_reviewed(false);
        p.now += 3600;
        let ask = ChangeResolveRequest { subject: request(&p, 263) };
        let (r, fx) = resolve(&mut p, &ask).unwrap();
        assert_eq!(r, Resolve::Pending);
        assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. })), "{fx:?}");
        assert!(p.resolving.contains(&mr_key(PROJECT, 263)));
        // Asked again while it runs: still pending, nothing more is sent.
        let (r, fx) = resolve(&mut p, &ask).unwrap();
        assert_eq!((r, fx.len()), (Resolve::Pending, 0));
        // It came back with an error.
        let f = p.mr_pages.get_mut(&mr_key(PROJECT, 263)).unwrap();
        f.loading = false;
        f.error = Some("no merge request !263".into());
        let (r, fx) = resolve(&mut p, &ask).unwrap();
        assert!(matches!(&r, Resolve::Failed(m) if m.contains("no merge request")), "{r:?}");
        assert!(fx.is_empty() && p.resolving.is_empty());
    }

    #[test]
    fn a_key_of_another_server_or_provider_is_not_answered() {
        let mut p = crate::demo::sample_reviewed(false);
        let other = SubjectKey::new("gitlab", "https://other.example.test", MERGE_REQUEST, "acme/team-a/api!263").unwrap();
        let wrong = SubjectKey::new("othertracker", &request(&p, 263).instance, MERGE_REQUEST, "acme/team-a/api!263").unwrap();
        for subject in [other, wrong] {
            assert!(resolve(&mut p, &ChangeResolveRequest { subject: subject.clone() }).is_none());
            assert!(agent_context(&p, &AgentContextRequest { subject }).is_none());
        }
        let ctx = agent_context(&p, &AgentContextRequest { subject: request(&p, 263) }).unwrap();
        assert!(ctx.fields.iter().any(|f| f.name == "head_sha"), "the held page's detail is used");
    }

    #[test]
    fn a_on_an_open_page_asks_the_host_for_the_brief_and_the_row_carries_the_shared_id() {
        let mut p = crate::demo::sample_reviewed(false);
        open_page(&mut p);
        let rows = rows(&p, Pane::Middle);
        assert_eq!(
            rows.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["", SEND_TO_AGENT_ACTION, REVIEW_LOCALLY_ACTION]
        );
        assert_eq!(rows[1].1.keys, "A");
        assert_eq!(rows[2].1.keys, "i");
        let pressed = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        let Some(Outcome::Handled(fx)) = key_press(&p, Pane::Middle, pressed) else { panic!("not handled") };
        let [Effect::SendToAgent(req)] = fx.as_slice() else { panic!("{fx:?}") };
        assert_eq!(req.subject, request(&p, 263));
        assert!(key_press(&p, Pane::Nav, pressed).is_none());
        assert!(rows_on_nav(&p).is_empty());
        // The plugin maps every pane of an open page to the page's own.
        assert_eq!(p.rows(Pane::Nav).iter().filter(|(id, _)| id == SEND_TO_AGENT_ACTION).count(), 1);
        assert!(matches!(
            p.on_key(Pane::Nav, pressed),
            Outcome::Handled(_)
        ));
        assert!(key_press(&p, Pane::Middle, KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)).is_none());
    }

    fn press(p: &GitlabPlugin, pane: Pane, c: char) -> Option<Vec<Effect>> {
        match key_press(p, pane, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)) {
            Some(Outcome::Handled(fx)) => Some(fx),
            Some(_) => panic!("only handled outcomes"),
            None => None,
        }
    }

    #[test]
    fn i_on_an_open_merge_request_starts_its_review_with_itself_as_the_change() {
        let mut p = crate::demo::sample_reviewed(false);
        open_page(&mut p);
        let fx = press(&p, Pane::Middle, 'i').unwrap();
        let [Effect::StartReview(r)] = fx.as_slice() else { panic!("{fx:?}") };
        assert_eq!(r.changes, [request(&p, 263)]);
        assert_eq!(r.origin, request(&p, 263));
        assert!(r.links.is_empty(), "the part names the change request itself");
    }

    #[test]
    fn a_merged_or_closed_merge_request_offers_no_review_and_says_so_on_i() {
        use crate::model::MrState;
        for state in [MrState::Merged, MrState::Closed] {
            let mut p = crate::demo::sample_reviewed(false);
            open_page(&mut p);
            for d in p.data.values_mut() {
                for m in d.mrs.iter_mut().filter(|m| m.iid == 263) {
                    m.state = state;
                }
            }
            let ids: Vec<String> = rows(&p, Pane::Middle).into_iter().map(|(id, _)| id).collect();
            assert_eq!(ids, ["", SEND_TO_AGENT_ACTION].map(String::from), "{state:?}");
            let fx = press(&p, Pane::Middle, 'i').unwrap();
            assert!(matches!(fx.as_slice(), [Effect::Notify(m)] if m.contains("not open")), "{fx:?}");
        }
    }

    #[test]
    fn the_review_key_is_not_the_nav_panes_or_another_modifiers() {
        let mut p = crate::demo::sample_reviewed(false);
        open_page(&mut p);
        assert!(press(&p, Pane::Nav, 'i').is_none());
        assert!(key_press(&p, Pane::Middle, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)).is_none());
        // `i` is the one key the row uses once: no other row of the menu takes it.
        let keys: Vec<String> = p.rows(Pane::Middle).into_iter().map(|(_, b)| b.keys).collect();
        assert_eq!(keys.iter().filter(|k| *k == "i").count(), 1, "{keys:?}");
    }

    fn rows_on_nav(p: &GitlabPlugin) -> Vec<Row> {
        rows(p, Pane::Nav)
    }

    #[test]
    fn the_context_names_the_merge_request_and_its_fields() {
        let mr = fixtures::mr(7, "ana", "acme/api");
        let c = context(BASE, &mr, Some(&detail())).unwrap();
        let has = |n: &str| c.fields.iter().any(|f| f.name == n);
        assert!(has("description") && has("author") && has("source_branch") && has("head_sha"));
        assert!(!has("title"), "the title is the context's own");
        assert_eq!(c.subject.id, "acme/api!7");
        assert_eq!(c.link.as_deref(), Some(mr.web_url.as_str()));
    }

    #[test]
    fn the_context_without_the_page_holds_the_row_only() {
        let c = context(BASE, &fixtures::mr(7, "ana", "acme/api"), None).unwrap();
        assert!(c.fields.iter().all(|f| f.name != "description" && f.name != "head_sha"));
        assert!(c.fields.iter().any(|f| f.name == "source_branch"));
    }

    #[test]
    fn open_threads_are_carried_and_bounded() {
        let mr = fixtures::mr(7, "ana", "acme/api");
        let mut d = detail();
        let one = d.discussions[0].clone();
        d.discussions = (0..MAX_THREADS + 3)
            .map(|_| Discussion { resolvable: true, resolved: false, ..one.clone() })
            .collect();
        let c = context(BASE, &mr, Some(&d)).unwrap();
        assert_eq!(c.fields.iter().find(|f| f.name == "open_threads").unwrap().value.lines().count(), MAX_THREADS);
        assert_eq!(c.fields.iter().find(|f| f.name == "more_open_threads").unwrap().value, "3");
        let r = resolution(BASE, &mr, &d).unwrap();
        assert_eq!(r.threads.len(), MAX_THREADS);
    }

    #[test]
    fn a_resolved_thread_is_not_open() {
        let mr = fixtures::mr(7, "ana", "acme/api");
        let mut d = detail();
        for t in &mut d.discussions {
            t.resolvable = true;
            t.resolved = true;
        }
        assert!(resolution(BASE, &mr, &d).unwrap().threads.is_empty());
    }

    #[test]
    fn the_resolution_names_the_head_sha_and_will_not_guess_it() {
        let mr = fixtures::mr(7, "ana", "acme/api");
        let r = resolution(BASE, &mr, &detail()).unwrap();
        assert_eq!(r.head_sha, "abc123");
        assert_eq!(r.repository, "acme/api");
        assert_eq!(r.source_branch, "feature/ABC-7");
        assert_eq!(r.target_branch, "main");
        let mut d = detail();
        d.refs = None;
        assert!(resolution(BASE, &mr, &d).is_none());
        d.refs = Some(DiffRefs::default());
        assert!(resolution(BASE, &mr, &d).is_none());
    }

    #[test]
    fn an_id_splits_into_project_and_iid() {
        assert_eq!(split_id("acme/team/api!12"), Some(("acme/team/api", 12)));
        assert_eq!(split_id("acme/api"), None);
        assert_eq!(split_id("acme/api!x"), None);
    }
}
