//! The tenancy page: a table of its repos (main's pipeline, open merge
//! requests, last activity) and a line about the hidden ones. `⏎` opens a
//! repo's page.

use radar_core::time;

use super::source;
use crate::GitlabPlugin;
use crate::model::Project;

pub(crate) struct RepoRow {
    pub project: Project,
    pub mrs: usize,
    /// RFC 3339: the later of the repo's activity and its newest merge request.
    pub latest: String,
}

/// The tenancy's shown repos, most recently active first.
pub(crate) fn repos(p: &GitlabPlugin, group: &str) -> Vec<RepoRow> {
    let Some(s) = source(p, group) else {
        return vec![];
    };
    let items = p.all_items();
    let mut rows: Vec<RepoRow> = p
        .data
        .get(&p.cfg.group_path(s))
        .map(|d| d.projects.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|pr| {
            p.cfg
                .source_for(&pr.full_path)
                .is_some_and(|(owner, _)| owner.group == s.group)
                && !p.cfg.is_hidden(&pr.full_path)
        })
        .map(|pr| {
            let mine: Vec<_> = items
                .iter()
                .filter(|it| it.mr.project_path == pr.full_path)
                .collect();
            let latest = mine
                .iter()
                .map(|it| it.mr.updated_at.clone())
                .chain([pr.last_activity.clone()])
                .max_by_key(|at| time::parse_rfc3339(at).unwrap_or(0))
                .unwrap_or_default();
            RepoRow {
                project: pr.clone(),
                mrs: mine.len(),
                latest,
            }
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(time::parse_rfc3339(&r.latest).unwrap_or(0)));
    rows
}

pub(super) fn tabs(p: &GitlabPlugin, group: &str) -> Vec<String> {
    vec![format!("Repos · {}", repos(p, group).len())]
}
