//! The repo page: its open merge requests (`⏎` opens one's page) and the
//! latest pipelines on its default branch.

use super::{branch_of, project};
use crate::model::{JobInfo, PipelineRun};
use crate::{GitlabPlugin, Item, why};

pub(crate) const MRS: usize = 0;
pub(crate) const PIPELINES: usize = 1;

/// The repo's open merge requests, whatever the view: what they need from
/// you first, then the most recently updated.
pub(super) fn mrs<'a>(p: &'a GitlabPlugin, path: &str) -> Vec<Item<'a>> {
    let mut items: Vec<Item<'a>> = p
        .all_items()
        .into_iter()
        .filter(|it| it.mr.project_path == path)
        .collect();
    items.sort_by(|a, b| {
        a.role
            .cmp(&b.role)
            .then_with(|| b.mr.updated_at.cmp(&a.mr.updated_at))
    });
    items
}

fn runs<'a>(p: &'a GitlabPlugin, path: &str) -> &'a [PipelineRun] {
    crate::runs_ownership::cached(p, path)
        .and_then(|f| f.value.as_deref())
        .unwrap_or_default()
}

pub(super) fn run<'a>(p: &'a GitlabPlugin, path: &str, row: usize) -> Option<&'a PipelineRun> {
    runs(p, path).get(row)
}

/// Main's pipeline, when it is red.
fn red(p: &GitlabPlugin, path: &str) -> Option<u64> {
    let pr = project(p, path);
    why::red(pr.main_pipeline, pr.main_pipeline_url.as_deref())
}

/// The failed jobs of main's red pipeline: they lead the pipelines tab.
pub(super) fn lead_rows(p: &GitlabPlugin, path: &str) -> Vec<JobInfo> {
    red(p, path).map_or_else(Vec::new, |pl| why::failed_rows(p, path, pl))
}

pub(super) fn len(p: &GitlabPlugin, path: &str, tab: usize) -> usize {
    if tab == PIPELINES {
        lead_rows(p, path).len() + runs(p, path).len()
    } else {
        mrs(p, path).len()
    }
}

pub(super) fn tabs(p: &GitlabPlugin, path: &str) -> Vec<String> {
    let pr = project(p, path);
    vec![
        "Merge requests".into(),
        format!("Pipelines on {}", branch_of(&pr)),
    ]
}

