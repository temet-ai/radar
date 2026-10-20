//! Tree selection and open groups on [`TreeNav`]: what the part's two trees
//! (the repositories and a merge request's changed files) need beyond it.

use std::collections::HashSet;

use radar_core::view::TreeNav;

/// A row's id and its children, nothing else: what [`open_new`] walks.
pub(crate) struct Skel {
    pub id: String,
    pub children: Vec<Skel>,
}

/// Open every node with children the first time it is seen (`seen` keeps
/// the ids down to each); after that, open and closed are the user's. With
/// nothing selected, the first row is.
pub(crate) fn open_new(nav: &mut TreeNav, nodes: &[Skel], seen: &mut HashSet<Vec<String>>) {
    fn walk(nav: &mut TreeNav, nodes: &[Skel], seen: &mut HashSet<Vec<String>>, path: &mut Vec<String>) {
        for n in nodes {
            path.push(n.id.clone());
            if !n.children.is_empty() {
                if seen.insert(path.clone()) {
                    nav.open(path.clone());
                }
                walk(nav, &n.children, seen, path);
            }
            path.pop();
        }
    }
    walk(nav, nodes, seen, &mut Vec::new());
    if nav.selected().is_empty()
        && let Some(first) = nodes.first()
    {
        nav.select(vec![first.id.clone()]);
    }
}

pub(crate) trait NavExt {
    /// Close the group at `path`.
    fn close(&mut self, path: &[String]);
    /// The open groups, sorted: a set, so two hosts send the same view.
    fn open_sorted(&self) -> Vec<Vec<String>>;
}

impl NavExt for TreeNav {
    fn close(&mut self, path: &[String]) {
        self.open.retain(|p| p.as_slice() != path);
    }

    fn open_sorted(&self) -> Vec<Vec<String>> {
        let mut open = self.open.clone();
        open.sort();
        open
    }
}
