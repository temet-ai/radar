//! The requested business target is transient and independent of journal redaction.
//! This is an application fence, not a change to the legacy public parsers.

use crate::{GitlabPlugin, client::MrDetailRead, config, pages};

pub(crate) const REJECTED: &str =
    "GitLab returned an unexpected merge request identity; page data was not updated";

#[derive(Debug, Clone)]
pub(crate) struct Requested {
    project: String,
    iid: u64,
}

impl Requested {
    pub(crate) fn new(project: &str, iid: u64) -> Self {
        Self {
            project: project.into(),
            iid,
        }
    }

    pub(crate) fn accepts(&self, key: &str, got: &MrDetailRead) -> bool {
        usable_path(&self.project)
            && key == pages::mr_key(&self.project, self.iid)
            && got.matches_iid(self.iid)
            && usable_path(&got.mr.project_path)
            && self.project.eq_ignore_ascii_case(&got.mr.project_path)
    }
}

// radar's existing supported ASCII path grammar, not full GitLab protocol validation.
// Do not repair, decode, trim or infer renamed project aliases here.
fn usable_path(path: &str) -> bool {
    config::group_path_problem(path).is_none()
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

/// Called only after the exact pending owner has been claimed. In particular, do
/// not use on_mr_page's Err branch: even that branch can continue a Changes read.
pub(crate) fn reject(p: &mut GitlabPlugin, key: &str) {
    if let Some(fetched) = p.mr_pages.get_mut(key) {
        fetched.loading = false;
        fetched.error = Some(REJECTED.into());
    }
}

#[cfg(test)]
mod tests;
