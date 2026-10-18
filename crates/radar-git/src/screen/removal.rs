//! Bind a removal to the observations shown in its question. This is not a
//! filesystem snapshot: status is a count and HEAD is the reader's short SHA.
use super::*;

const CHANGED: &str = "worktree changed: press d to review removal again";
const EXPIRED: &str = "removal expired: press d to review it again";
const UNAVAILABLE: &str =
    "worktree status is unavailable: read the clones again before removing it";

#[derive(Debug, Default)]
pub(super) struct State {
    serial: u64,
    pub(super) pending: Option<Intent>,
}

pub(super) struct Intent {
    menu: String,
    clone: String,
    tree: String,
    branch: String,
    head: String,
    dirty: usize,
    force: bool,
    remote: String,
    project: String,
    roots: Vec<String>,
    depth: usize,
    links: Vec<local::Link>,
}

// Scope and paths are held only for validation, not emitted as debug or journal data.
impl std::fmt::Debug for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Intent { .. }")
    }
}

fn current<'a>(
    p: &'a GitPlugin,
    clone: &str,
    tree: &str,
) -> Result<(&'a Checkout, &'a Tree), &'static str> {
    let checkout = checkout_of(p, clone)
        .filter(|c| c.main.is_empty())
        .ok_or(CHANGED)?;
    let detail = p.branches.details.get(clone).ok_or(CHANGED)?;
    if detail.dir != clone || !detail.error.is_empty() {
        return Err(CHANGED);
    }
    if tree == clone || clones(p).iter().any(|c| c.dir == tree) {
        return Err("that is the clone itself");
    }
    let mut matches = detail.trees.iter().filter(|t| t.dir == tree);
    let tree = matches.next().ok_or(CHANGED)?;
    if matches.next().is_some() {
        return Err(CHANGED);
    }
    if tree.status != StatusRead::Current {
        return Err(UNAVAILABLE);
    }
    Ok((checkout, tree))
}

impl Intent {
    fn validate(&self, p: &GitPlugin) -> Result<(), &'static str> {
        let (checkout, tree) = current(p, &self.clone, &self.tree)?;
        if tree.branch != self.branch
            || tree.head != self.head
            || tree.dirty != self.dirty
            || checkout.remote != self.remote
            || checkout.project != self.project
            || p.cfg.roots != self.roots
            || p.cfg.depth != self.depth
            || p.cfg.links != self.links
            || p.branches.changing.contains(&self.clone)
        {
            return Err(CHANGED);
        }
        Ok(())
    }
}

pub(super) fn ask(p: &mut GitPlugin) -> Vec<Effect> {
    let Some(Sel::Tree(detail, selected)) = selected(p) else {
        return vec![Effect::Notify(EXPIRED.into())];
    };
    if p.branches.tree.borrow().selected().first() != Some(&clone_id(&detail.dir)) {
        return vec![Effect::Notify(CHANGED.into())];
    }
    let (checkout, tree) = match current(p, &detail.dir, &selected.dir) {
        Ok(target) => target,
        Err(why) => return vec![Effect::error(None, why)],
    };
    let Some(serial) = p.branches.removal.serial.checked_add(1) else {
        return vec![Effect::Notify(EXPIRED.into())];
    };
    // An old answer cannot authorize a newly opened question, even at the same path.
    let menu = format!("{DELETE}:tree\t{serial}");
    let intent = Intent {
        menu: menu.clone(),
        clone: detail.dir.clone(),
        tree: tree.dir.clone(),
        branch: tree.branch.clone(),
        head: tree.head.clone(),
        dirty: tree.dirty,
        force: tree.dirty > 0,
        remote: checkout.remote.clone(),
        project: checkout.project.clone(),
        roots: p.cfg.roots.clone(),
        depth: p.cfg.depth,
        links: p.cfg.links.clone(),
    };
    let question = super::ask(
        menu,
        format!("remove the worktree {}?", base_name(&tree.dir)),
        lost_tree(tree),
        "remove it",
        "keep it",
    );
    p.branches.removal.serial = serial;
    p.branches.removal.pending = Some(intent);
    vec![question]
}

fn blocked(why: &str) -> Vec<Effect> {
    vec![
        Effect::Report(JournalOutcome::Blocked),
        Effect::error(None, why),
    ]
}

pub(super) fn answer(p: &mut GitPlugin, menu: &str, value: &str) -> Vec<Effect> {
    answer_with(p, menu, value, prune::remove_tree)
}

fn answer_with(
    p: &mut GitPlugin,
    menu: &str,
    value: &str,
    remove: impl FnOnce(&Path, &str, bool) -> Result<String, String> + Send + 'static,
) -> Vec<Effect> {
    if !p
        .branches
        .removal
        .pending
        .as_ref()
        .is_some_and(|intent| intent.menu == menu)
    {
        return blocked(EXPIRED);
    }
    // Matching answers consume their own intent even when refused or cancelled.
    let intent = p.branches.removal.pending.take().expect("matched intent");
    if value != "yes" {
        return vec![Effect::Report(JournalOutcome::Cancelled)];
    }
    if let Err(why) = intent.validate(p) {
        return blocked(why);
    }
    p.branches.changing.insert(intent.clone.clone());
    vec![
        journal_target("git.worktree.delete", &intent.clone, "", &intent.tree),
        Effect::Report(JournalOutcome::Confirmed),
        Effect::Report(JournalOutcome::Started),
        blocking(move || {
            let said = remove(Path::new(&intent.clone), &intent.tree, intent.force);
            BranchMsg::Done {
                said,
                reread: vec![intent.clone],
            }
        }),
    ]
}

#[cfg(test)]
#[path = "removal_tests.rs"]
mod tests;
