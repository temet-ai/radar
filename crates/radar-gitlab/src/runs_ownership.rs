//! Session-local repository pipeline cache scope and one-use read ownership.
//! Neither connection generations nor request IDs enter persisted history.

use crate::{
    GitlabPlugin, model::PipelineRun, page_ownership, pages, write_context::ConnectionStamp,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub path: String,
    pub branch: String,
    pub owner: page_ownership::Read,
}

#[derive(Debug)]
pub(crate) struct Scope {
    branch: String,
    connection: ConnectionStamp,
    pending: Option<page_ownership::Read>,
}

fn current(p: &GitlabPlugin, path: &str, scope: &Scope) -> bool {
    p.is_current_connection(scope.connection)
        && scope.branch == pages::branch_of(&pages::project(p, path))
}

/// All readers, including row activation, must use the same exact-ref fence.
pub(crate) fn cached<'a>(
    p: &'a GitlabPlugin,
    path: &str,
) -> Option<&'a pages::Fetched<Vec<PipelineRun>>> {
    p.runs_scopes
        .get(path)
        .filter(|scope| current(p, path, scope))?;
    p.runs.get(path)
}

pub(crate) fn invalidate(p: &mut GitlabPlugin) {
    p.runs.clear();
    p.runs_scopes.clear();
}

/// Retire changed refs immediately on accepted source replacement, including
/// A-B-A changes without a page visit. Unrelated repositories remain independent.
pub(crate) fn reconcile(p: &mut GitlabPlugin) {
    let retired: Vec<_> = p
        .runs_scopes
        .iter()
        .filter(|(path, scope)| !current(p, path, scope))
        .map(|(path, _)| path.clone())
        .collect();
    for path in retired {
        p.runs_scopes.remove(&path);
        p.runs.remove(&path);
    }
}

pub(crate) fn begin(p: &mut GitlabPlugin, path: &str, branch: &str) -> Request {
    reconcile(p);
    // A bare cache without scope cannot be re-labelled with today's branch.
    if !p.runs_scopes.contains_key(path) {
        p.runs.remove(path);
    }
    let owner = page_ownership::next(p);
    p.runs_scopes.insert(
        path.into(),
        Scope {
            branch: branch.into(),
            connection: owner.connection,
            pending: Some(owner),
        },
    );
    p.runs.entry(path.into()).or_default().loading = true;
    Request {
        path: path.into(),
        branch: branch.into(),
        owner,
    }
}

/// Consume this owner even when its independently retired cache is absent.
/// A duplicate or superseded reply never consumes the newer pending slot.
pub(crate) fn claim(p: &mut GitlabPlugin, request: &Request) -> bool {
    let Some(scope) = p.runs_scopes.get(&request.path) else {
        return false;
    };
    if !current(p, &request.path, scope)
        || scope.branch != request.branch
        || scope.pending != Some(request.owner)
    {
        return false;
    }
    p.runs_scopes
        .get_mut(&request.path)
        .expect("checked scope")
        .pending = None;
    p.runs.contains_key(&request.path)
}

#[cfg(test)]
pub(crate) fn pending(p: &GitlabPlugin, path: &str) -> Request {
    let scope = &p.runs_scopes[path];
    Request {
        path: path.into(),
        branch: scope.branch.clone(),
        owner: scope.pending.unwrap(),
    }
}

#[cfg(test)]
mod tests;
