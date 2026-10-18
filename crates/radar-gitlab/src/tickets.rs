//! What GitLab adds to tickets, a kind another part owns, knowing only the
//! shared vocabulary: a `ticket` subject's id is its key (`ABC-12`). The
//! merge requests whose title or branch carry the key, read from GitLab's
//! own cache (never fetched here), give:
//!
//! - a Code tab on the ticket's page: each merge request with its state,
//!   pipeline and approvals; `⏎` on one shows its page;
//! - a `code` column on lists of tickets: `!263 review`, `2 MRs · 1 approved`;
//! - a Merge requests section in the ticket's detail pane and overview;
//! - related repo subjects, once per full project path.
//!
//! A ticket no merge request carries gets nothing.

use std::collections::BTreeSet;

use radar_core::view::Text;
use radar_core::{Adds, Contribution, Point, Row, Subject};

use crate::GitlabPlugin;
use crate::join;
use crate::model::{Mr, MrState};

const TICKET: &str = "ticket";

pub(crate) const ADDS: [Adds; 1] = [Adds::new(
    TICKET,
    "a Code tab, a code column and merge requests in the detail pane",
)];

/// Whether `text` has `key` in it as a whole word, in any case
/// (`feature/abc-12-stats` carries `ABC-12`; `ABC-123` does not).
fn carries(text: &str, key: &str) -> bool {
    let (text, key) = (text.to_ascii_uppercase(), key.to_ascii_uppercase());
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(at) = text[from..].find(&key) {
        let start = from + at;
        let end = start + key.len();
        let before = text[..start].chars().next_back().is_none_or(|c| !word(c));
        let after = text[end..].chars().next().is_none_or(|c| !word(c));
        if before && after {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The merge requests held that carry `key`: open ones first, newest first.
pub(crate) fn carrying<'a>(p: &'a GitlabPlugin, key: &str) -> Vec<&'a Mr> {
    let mut mrs: Vec<&Mr> = p
        .data
        .values()
        .flat_map(|d| d.mrs.iter())
        .filter(|m| carries(&m.title, key) || carries(&m.source_branch, key))
        .collect();
    mrs.sort_by(|a, b| {
        (a.state != MrState::Opened)
            .cmp(&(b.state != MrState::Opened))
            .then(b.updated_at.cmp(&a.updated_at))
    });
    mrs.dedup_by(|a, b| a.web_url == b.web_url);
    mrs
}

/// Repos linked by the same cached merge requests shown on a ticket, including
/// merged and closed ones. Keep full project paths distinct, even with the same
/// short repo name or several merge requests across overlapping tenancies.
pub(crate) fn related(p: &GitlabPlugin, s: &Subject) -> Vec<Subject> {
    if !s.is(TICKET) || s.id.is_empty() {
        return Vec::new();
    }
    let mut projects = BTreeSet::new();
    carrying(p, &s.id)
        .into_iter()
        .filter(|m| projects.insert(m.project_path.as_str()))
        .map(|m| join::repo(p, &m.project_path))
        .collect()
}

/// `open · review`, `merged`, `draft`.
fn state(p: &GitlabPlugin, m: &Mr) -> String {
    match m.state {
        MrState::Opened => m.role(&p.me).word().to_string(),
        s => s.word().to_string(),
    }
}

fn pipeline(m: &Mr) -> String {
    match m.pipeline {
        Some(pl) => format!("pipeline {}", pl.word()),
        None => "no pipeline".into(),
    }
}

fn approvals(m: &Mr) -> String {
    if m.approved {
        radar_core::writes::done_line(&format!("approved ({})", m.approvals_words()))
    } else {
        format!("approvals {}", m.approvals_words())
    }
}

fn name(m: &Mr) -> String {
    let repo = m.project_path.rsplit('/').next().unwrap_or(&m.project_path);
    format!("{repo} !{}", m.iid)
}

/// What GitLab puts at `point` on a ticket.
pub(crate) fn contribute(p: &GitlabPlugin, point: Point, s: &Subject) -> Option<Contribution> {
    if !s.is(TICKET) {
        return None;
    }
    let mrs = carrying(p, &s.id);
    if mrs.is_empty() {
        return None;
    }
    Some(match point {
        Point::Column => {
            let value = match mrs.as_slice() {
                [m] => format!("!{} {}", m.iid, state(p, m)),
                _ => {
                    let open = mrs.iter().filter(|m| m.state == MrState::Opened).count();
                    let approved = mrs.iter().filter(|m| m.state == MrState::Opened && m.approved).count();
                    let mut v = format!("{} MRs", mrs.len());
                    if approved > 0 {
                        v.push_str(&format!(" · {approved} approved"));
                    } else if open < mrs.len() {
                        v.push_str(&format!(" · {open} open"));
                    }
                    v
                }
            };
            Contribution::Cell(radar_core::Cell::new("code", value))
        }
        Point::Section => Contribution::Section {
            title: "Merge requests".into(),
            lines: mrs
                .iter()
                .flat_map(|m| {
                    [
                        Text::plain(format!("{} · {} · {} · {}", name(m), state(p, m), pipeline(m), approvals(m))),
                        Text::plain(format!("  {}", m.title)),
                    ]
                })
                .collect(),
        },
        Point::Tab => Contribution::Tab {
            title: "Code".into(),
            rows: mrs
                .iter()
                .map(|m| Row {
                    line: Text::plain(format!(
                        "{:<16} {:<8} {:<18} {:<22} {}",
                        name(m),
                        state(p, m),
                        pipeline(m),
                        approvals(m),
                        m.title
                    )),
                    subject: Some(join::of_mr(m)),
                })
                .collect(),
        },
        Point::Chips | Point::Actions | Point::Line | Point::Mark => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::mr;
    use crate::tests::sample;
    use radar_core::Plugin;

    fn ticket(key: &str) -> Subject {
        Subject::new(TICKET, key, key).with("title", "anything")
    }

    #[test]
    fn a_key_is_carried_as_a_whole_word_in_title_or_branch() {
        assert!(carries("ABC-12: Add a stats endpoint", "ABC-12"));
        assert!(carries("feature/abc-12-stats", "ABC-12"));
        assert!(!carries("ABC-123: other", "ABC-12"));
        assert!(!carries("XABC-12", "ABC-12"));
    }

    fn cached(mrs: Vec<Mr>) -> GitlabPlugin {
        let mut p = GitlabPlugin::default();
        // Related subjects need no credentials, project listing or live service.
        p.cfg.url = "https://code.example.invalid/".into();
        p.cfg.token.clear();
        p.data.insert(
            "acme".into(),
            crate::GroupData {
                mrs,
                ..Default::default()
            },
        );
        p
    }

    #[test]
    fn related_repos_are_cached_full_paths_once_each_across_merge_requests_and_tenancies() {
        let mut first = mr(21, "alice", "acme/team-a/api");
        first.title = "ABC-12: add stats".into();
        let mut second = mr(22, "bob", "acme/team-a/api");
        second.source_branch = "feature/abc-12-tests".into();
        let mut other = mr(23, "alice", "acme/team-b/api");
        other.title = "ABC-12: use stats".into();
        let mut p = cached(vec![first.clone(), other, second]);
        p.data.insert(
            "acme/team-a".into(),
            crate::GroupData {
                mrs: vec![first],
                ..Default::default()
            },
        );

        let repos = p.related(&ticket("abc-12"));
        assert_eq!(repos.len(), 2);
        for (s, project) in repos.iter().zip(["acme/team-a/api", "acme/team-b/api"]) {
            assert_eq!(s.kind, "repo");
            assert_eq!(s.id, project);
            assert_eq!(s.field("project"), Some(project));
            assert_eq!(s.label, "api");
            assert_eq!(
                s.url,
                Some(format!("https://code.example.invalid/{project}"))
            );
        }
    }

    #[test]
    fn related_repos_match_whole_ticket_keys_in_titles_or_branches() {
        let mrs = [
            ("acme/title", "ABC-12: stats", "feature/stats"),
            ("acme/branch", "stats tests", "feature/abc-12-tests"),
            ("acme/longer-title", "ABC-123: other", "feature/other"),
            ("acme/longer-branch", "other", "feature/ABC-123"),
            ("acme/prefix", "XABC-12: other", "feature/other"),
            ("acme/suffix", "other", "feature/ABC-12_extra"),
        ]
        .into_iter()
        .map(|(project, title, branch)| {
            let mut m = mr(21, "alice", project);
            m.title = title.into();
            m.source_branch = branch.into();
            m
        })
        .collect();
        let p = cached(mrs);
        let repos: Vec<_> = p
            .related(&ticket("ABC-12"))
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(repos, ["acme/title", "acme/branch"]);
    }

    #[test]
    fn related_repos_include_cached_merged_and_closed_merge_requests() {
        let mrs = [
            ("acme/merged", MrState::Merged),
            ("acme/closed", MrState::Closed),
            ("acme/open", MrState::Opened),
        ]
        .into_iter()
        .map(|(project, state)| {
            let mut m = mr(12, "alice", project);
            m.state = state;
            m
        })
        .collect();
        let p = cached(mrs);
        let repos: Vec<_> = p
            .related(&ticket("ABC-12"))
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(repos, ["acme/open", "acme/merged", "acme/closed"]);
    }

    #[test]
    fn unknown_empty_and_non_ticket_subjects_have_no_related_repos() {
        let p = cached(vec![mr(12, "alice", "acme/team-a/api")]);
        assert!(p.related(&ticket("ABC-99")).is_empty());
        assert!(p.related(&ticket("")).is_empty());
        assert!(
            p.related(&Subject::new("thread", "ABC-12", "ABC-12"))
                .is_empty()
        );
        assert!(cached(vec![]).related(&ticket("ABC-12")).is_empty());
    }

    #[test]
    fn a_ticket_gets_its_merge_requests_as_a_tab_a_column_and_a_section() {
        let p = sample();
        // !263 (open) and !150 (merged) carry ABC-12 or ABC-9 in the sample.
        let Some(Contribution::Tab { title, rows }) = p.contribute(Point::Tab, &ticket("ABC-12")) else {
            panic!("a tab")
        };
        assert_eq!(title, "Code");
        assert_eq!(rows.len(), 1);
        let line = rows[0].line.to_string();
        assert!(line.starts_with("api !263") && line.contains("approvals"), "{line}");
        let s = rows[0].subject.as_ref().unwrap();
        assert_eq!((s.kind.as_str(), s.id.as_str()), ("merge-request", "acme/team-a/api!263"));
        let Some(Contribution::Cell(cell)) = p.contribute(Point::Column, &ticket("ABC-12")) else {
            panic!("a cell")
        };
        assert_eq!(cell.heading, "code");
        assert!(cell.text.starts_with("!263 "), "{cell:?}");
        let Some(Contribution::Section { title, lines }) = p.contribute(Point::Section, &ticket("abc-12")) else {
            panic!("a section")
        };
        assert_eq!(title, "Merge requests");
        assert!(lines[0].to_string().starts_with("api !263 · "), "{lines:?}");
        // Nothing for a ticket no merge request carries, or for chips and actions.
        assert!(p.contribute(Point::Tab, &ticket("ABC-99")).is_none());
        assert!(p.contribute(Point::Chips, &ticket("ABC-12")).is_none());
        assert!(p.contribute(Point::Actions, &ticket("ABC-12")).is_none());
        // Nor for other kinds.
        let other = Subject::new("thread", "ABC-12", "ABC-12");
        assert!(p.contribute(Point::Tab, &other).is_none());
    }
}
