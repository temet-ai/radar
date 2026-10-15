//! How GitLab joins the other parts (radar-core's `compose`): the subjects it
//! owns (merge requests, repos, pipelines), recognising their links in text,
//! showing them when another part asks, and the places it offers others on
//! its rows, lists and pages.
//!
//! Its subjects, in the vocabulary the parts share (GUIDE.md):
//! - `merge-request`: id `<project>!<iid>`; fields `project`, `iid`,
//!   `title`, `branch`, `target`, `author`, `state`;
//! - `repo`: id the project's full path; field `project`;
//! - `pipeline`: id `<project>#<id>`; fields `project`, `id`;
//! - `job`: id `<project>#<id>` (the job's id); fields `project`, `id`,
//!   and `name`, `stage`, `status`, `pipeline` once known.

mod target;
pub(crate) use target::Target;

use std::ops::Range;

use radar_core::plugin::{Effect, Pane};
use radar_core::{Offer, Owned, Parts, Point, Showing, Subject};

use crate::GitlabPlugin;
use crate::model::Mr;
use crate::pages::{self, Kind};
use crate::screen::{self, Selected};

pub(crate) const MERGE_REQUEST: &str = "merge-request";
pub(crate) const REPO: &str = "repo";
pub(crate) const PIPELINE: &str = "pipeline";
pub(crate) const JOB: &str = "job";

/// A pipeline is shown by its link; merge requests, repos and jobs have pages.
pub(crate) const OWNS: [Owned; 4] = [
    Owned::page(MERGE_REQUEST, "merge requests"),
    Owned::page(REPO, "repos"),
    Owned::link(PIPELINE, "pipelines"),
    Owned::page(JOB, "jobs"),
];

pub(crate) const OFFERS: [Offer; 13] = [
    Offer::new(Point::Chips, MERGE_REQUEST, "merge request rows"),
    Offer::new(Point::Chips, REPO, "repo rows and page"),
    Offer::new(Point::Column, MERGE_REQUEST, "merge request lists"),
    Offer::new(Point::Column, REPO, "the repositories middle pane"),
    Offer::new(
        Point::Section,
        MERGE_REQUEST,
        "the merge request page and detail pane",
    ),
    Offer::new(Point::Tab, MERGE_REQUEST, "the merge request page"),
    Offer::new(Point::Tab, REPO, "the repo page"),
    Offer::new(Point::Actions, MERGE_REQUEST, "merge requests"),
    Offer::new(Point::Actions, REPO, "repos"),
    Offer::new(Point::Actions, JOB, "the job page"),
    Offer::new(Point::Line, MERGE_REQUEST, "the merge request page, under its title"),
    Offer::new(Point::Line, REPO, "the repo page, under its title"),
    Offer::new(Point::Mark, REPO, "repo rows: the icon and where it is"),
];

/// What some text links to, once each, in order: a link written in
/// markdown as what a part recognises in its address (a merge request's
/// page), else the link itself named by its words; then what the parts
/// recognise in `texts` (a ticket's key, a thread's link) and any other
/// address written out. Showing one opens its page here, or the link.
pub(crate) fn links_in(
    parts: &Parts<'_>,
    written: &[radar_core::ui::rich::Link],
    texts: &[String],
) -> Vec<Subject> {
    let mut out: Vec<Subject> = Vec::new();
    let mut add = |s: Subject| {
        if !out.iter().any(|o| o.kind == s.kind && o.id == s.id) {
            out.push(s);
        }
    };
    for l in written {
        match parts
            .recognise(&l.url)
            .into_iter()
            .find(|(r, _)| r.start == 0 && r.end == l.url.len())
        {
            Some((_, mut s)) => {
                s.url = Some(l.url.clone());
                add(s);
            }
            None => {
                let mut s = Subject::link(l.url.clone());
                if !l.text.trim().is_empty() {
                    s.label = l.text.clone();
                }
                add(s);
            }
        }
    }
    for text in texts {
        for (_, s) in parts.recognise(text) {
            add(s);
        }
        for (_, url) in addresses_in(text) {
            if !parts
                .recognise(url)
                .iter()
                .any(|(r, _)| r.start == 0 && r.end == url.len())
            {
                add(Subject::link(url));
            }
        }
    }
    out
}

/// Addresses written out in `text` (`https://…`), trailing punctuation off.
fn addresses_in(text: &str) -> Vec<(Range<usize>, &str)> {
    let delimiter = |c: char| c.is_whitespace() || "<>\"'".contains(c);
    let mut from = 0;
    text.split_inclusive(delimiter)
        .filter_map(|chunk| {
            let offset = from;
            from += chunk.len();
            // Only the first address in a contiguous token can be its URL.
            // A foreign URL's path/query/fragment may itself contain `https://`.
            let colon = chunk.find("://")?;
            let start = [5, 4].into_iter().find_map(|len| {
                let start = colon.checked_sub(len)?;
                let scheme = chunk.get(start..colon)?;
                (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
                    .then_some(start)
            })?;
            if start > 0 && !chunk[..start].ends_with(['(', '[']) {
                return None;
            }
            let mut url = chunk[start..]
                .trim_end_matches(delimiter)
                .trim_end_matches(['.', ',', ';', ':', '!', '?', '*']);
            // Strip enclosing prose punctuation, but keep balanced URL syntax
            // such as an IPv6 authority or parentheses in a query/fragment.
            loop {
                let pair = if url.ends_with(')') {
                    Some(('(', ')'))
                } else if url.ends_with(']') {
                    Some(('[', ']'))
                } else {
                    None
                };
                let Some((open, close)) = pair else { break };
                if url.matches(close).count() <= url.matches(open).count() {
                    break;
                }
                url = url[..url.len() - 1].trim_end_matches(['.', ',', ';', ':', '!', '?', '*']);
            }
            Some((offset + start..offset + start + url.len(), url))
        })
        .collect()
}

#[cfg(test)]
mod links_tests {
    use super::*;

    #[test]
    fn links_in_text_are_its_written_links_once_each_then_addresses_written_out() {
        let parts = Parts::none();
        let written = vec![
            radar_core::ui::rich::Link { text: "the lib".into(), url: "https://x.example/lib".into() },
            radar_core::ui::rich::Link { text: "https://x.example/lib".into(), url: "https://x.example/lib".into() },
        ];
        let texts = vec![
            "See https://x.example/lib, then (https://x.example/notes) and nothing else.".to_string(),
            "A ticket ABC-12 no part here knows.".to_string(),
        ];
        let got: Vec<(String, String)> = links_in(&parts, &written, &texts)
            .into_iter()
            .map(|s| (s.label, s.url.unwrap_or_default()))
            .collect();
        assert_eq!(
            got,
            [
                ("the lib".to_string(), "https://x.example/lib".to_string()),
                ("https://x.example/notes".to_string(), "https://x.example/notes".to_string()),
            ]
        );
    }
}

fn last(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The key a merge request's row goes by for motion: its subject's.
pub(crate) fn motion_key(mr: &Mr) -> String {
    radar_core::motion::key(MERGE_REQUEST, &format!("{}!{}", mr.project_path, mr.iid))
}

/// A merge request held, as a subject, with what others match on.
pub(crate) fn of_mr(mr: &Mr) -> Subject {
    let s = Subject::new(
        MERGE_REQUEST,
        format!("{}!{}", mr.project_path, mr.iid),
        format!("{} !{}", last(&mr.project_path), mr.iid),
    )
    .with_url(mr.web_url.clone())
    .with("project", mr.project_path.clone())
    .with("iid", mr.iid.to_string())
    .with("title", mr.title.clone())
    .with("branch", mr.source_branch.clone())
    .with("target", mr.target_branch.clone())
    .with("author", mr.author.username.clone())
    .with("state", mr.state.word());
    // What a review of this MR is of (the sessions chip reads it to say stale).
    if mr.head_sha.is_empty() { s } else { s.with("head_sha", mr.head_sha.clone()) }
}

/// A merge request by where it is: from what is held, else only its link.
pub(crate) fn mr(p: &GitlabPlugin, project: &str, iid: u64) -> Subject {
    match pages::find_mr(p, project, iid) {
        Some(m) => of_mr(m),
        None => Subject::new(
            MERGE_REQUEST,
            format!("{project}!{iid}"),
            format!("{} !{iid}", last(project)),
        )
        .with_url(format!(
            "{}/{project}/-/merge_requests/{iid}",
            p.cfg.url.trim_end_matches('/')
        ))
        .with("project", project)
        .with("iid", iid.to_string()),
    }
}

pub(crate) fn repo(p: &GitlabPlugin, project: &str) -> Subject {
    Subject::new(REPO, project, last(project))
        .with_url(format!("{}/{project}", p.cfg.url.trim_end_matches('/')))
        .with("project", project)
}

fn pipeline(p: &GitlabPlugin, project: &str, id: u64) -> Subject {
    Subject::new(
        PIPELINE,
        format!("{project}#{id}"),
        format!("pipeline #{id}"),
    )
    .with_url(format!(
        "{}/{project}/-/pipelines/{id}",
        p.cfg.url.trim_end_matches('/')
    ))
    .with("project", project)
    .with("id", id.to_string())
}

/// A job by its id: with its name, stage and status once they are known.
pub(crate) fn job(p: &GitlabPlugin, project: &str, id: u64) -> Subject {
    let base = p.cfg.url.trim_end_matches('/');
    let s = Subject::new(JOB, format!("{project}#{id}"), format!("job #{id}"))
        .with_url(format!("{base}/{project}/-/jobs/{id}"))
        .with("project", project)
        .with("id", id.to_string());
    match p.jobs.get(&crate::log_ownership::JobKey::new(project, id)) {
        Some(j) => {
            let mut s = s
                .with("name", j.name.clone())
                .with("stage", j.stage.clone())
                .with("status", j.status.clone());
            s.label = format!("{} #{id}", j.name);
            if j.pipeline > 0 {
                s = s.with("pipeline", j.pipeline.to_string());
            }
            s
        }
        None => s,
    }
}

/// What a page is about, as a subject (a tenancy is not one).
pub(crate) fn of_page(p: &GitlabPlugin, kind: &Kind) -> Option<Subject> {
    match kind {
        Kind::Mr { project, iid } => Some(mr(p, project, *iid)),
        Kind::Repo(path) => Some(repo(p, path)),
        Kind::Job { project, id } => Some(job(p, project, *id)),
        Kind::Tenancy(_) => None,
    }
}

/// What the focused pane is about: the page, or the tree's selection.
pub(crate) fn subject(p: &GitlabPlugin, pane: Pane) -> Option<Subject> {
    if let Some(page) = p.pages.last() {
        return of_page(p, &page.kind);
    }
    if pane == Pane::Nav {
        return None;
    }
    match screen::selected(p, p.visible())? {
        Selected::Mr(it) => Some(of_mr(it.mr)),
        Selected::Repo(pr, _) => Some(repo(p, &pr.full_path)),
        Selected::Tenancy(_) => None,
    }
}

/// Links to this GitLab in `text`: merge requests, pipelines, and repos it
/// knows. Each range covers the whole link.
pub(crate) fn recognise(p: &GitlabPlugin, text: &str) -> Vec<(Range<usize>, Subject)> {
    addresses_in(text)
        .into_iter()
        .filter_map(|(range, url)| {
            let target = Target::from_url(&p.cfg.url, url)?;
            let project = &target.project;
            let mut subject = match target.kind {
                MERGE_REQUEST => {
                    let iid = target.number?;
                    let s = Subject::new(
                        MERGE_REQUEST,
                        format!("{project}!{iid}"),
                        format!("{} !{iid}", last(project)),
                    )
                    .with("project", project)
                    .with("iid", iid.to_string());
                    match pages::find_mr(p, project, iid) {
                        Some(m) => s
                            .with("title", m.title.clone())
                            .with("branch", m.source_branch.clone())
                            .with("target", m.target_branch.clone())
                            .with("author", m.author.username.clone())
                            .with("state", m.state.word()),
                        None => s,
                    }
                }
                PIPELINE => pipeline(p, project, target.number?),
                JOB => job(p, project, target.number?),
                REPO => {
                    let known = p
                        .data
                        .values()
                        .flat_map(|d| d.projects.iter())
                        .any(|pr| pr.full_path == *project);
                    if !known {
                        return None;
                    }
                    repo(p, project)
                }
                _ => return None,
            };
            subject.url = Some(url.into());
            Some((range, subject))
        })
        .collect()
}

/// Another part asked to show one of ours: its page, on top of whatever
/// pages were open (so `esc` leaves them as they were).
pub(crate) fn show(p: &mut GitlabPlugin, subject: &Subject) -> Showing {
    // Pipelines remain external links, never a local page/read request.
    if subject.kind == PIPELINE {
        return match &subject.url {
            Some(url) => Showing::Done(vec![Effect::Open(url.clone())]),
            None => Showing::Cannot,
        };
    }
    let Some(target) = Target::consistent(&p.cfg.url, subject) else {
        return Showing::Cannot;
    };
    if subject.field("project").is_none()
        || target
            .number_field()
            .is_some_and(|field| subject.field(field).is_none())
    {
        return Showing::Cannot;
    }
    let kind = match (target.kind, target.number) {
        (MERGE_REQUEST, Some(iid)) => Kind::Mr {
            project: target.project,
            iid,
        },
        (REPO, None) => Kind::Repo(target.project),
        (JOB, Some(id)) => Kind::Job {
            project: target.project,
            id,
        },
        _ => return Showing::Cannot,
    };
    let mut effects = vec![Effect::Focus(Pane::Middle)];
    effects.extend(pages::push(p, kind));
    Showing::Page {
        screen: crate::SCREEN.id,
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample;

    #[test]
    fn recognises_its_own_links_and_only_known_repos() {
        let p = sample();
        let text = "see https://gitlab.com/acme/team-a/api/-/merge_requests/263#note_1, \
                    https://gitlab.com/acme/team-a/api/-/pipelines/99 and \
                    https://gitlab.com/acme/team-a/web/ but not https://gitlab.com/acme/nope; \
                    it broke in https://gitlab.com/acme/team-a/web/-/jobs/5001.";
        let found = recognise(&p, text);
        let ids: Vec<(&str, &str)> = found
            .iter()
            .map(|(_, s)| (s.kind.as_str(), s.id.as_str()))
            .collect();
        assert_eq!(
            ids,
            [
                (MERGE_REQUEST, "acme/team-a/api!263"),
                (PIPELINE, "acme/team-a/api#99"),
                (REPO, "acme/team-a/web"),
                (JOB, "acme/team-a/web#5001"),
            ]
        );
        // A job the sample knows carries its name, and shows as its page.
        assert_eq!(found[3].1.field("name"), Some("unit-test"));
        assert_eq!(found[3].1.label, "unit-test #5001");
        let mut p = sample();
        let Showing::Page { effects, .. } = show(&mut p, &found[3].1) else {
            panic!("a job has a page");
        };
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Spawn(_))),
            "its log is held already: not fetched again"
        );
        assert_eq!(
            p.pages.last().map(|pg| pg.kind.clone()),
            Some(Kind::Job { project: "acme/team-a/web".into(), id: 5001 })
        );
        assert_eq!(
            found[0].1.field("title"),
            Some("ABC-12: Add a stats endpoint for the dashboard")
        );
        assert!(text[found[0].0.clone()].ends_with("263#note_1"), "no comma");
    }

    #[test]
    fn a_merge_request_subject_carries_its_head_sha_when_known() {
        let mut mr = crate::model::fixtures::mr(7, "alice", "g/api");
        assert_eq!(of_mr(&mr).field("head_sha"), None, "an old cache has none");
        mr.head_sha = "abc123".into();
        assert_eq!(of_mr(&mr).field("head_sha"), Some("abc123"));
    }
}

#[cfg(test)]
mod routing_tests;
