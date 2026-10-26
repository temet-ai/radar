//! What a repo or merge request page can do beyond showing: rebase and
//! merge on GitLab, each in the page's actions menu (`x`) and on its own
//! key, asked first. What can be done on this machine (a checkout, an
//! editor, a terminal) is another part's, added to the page's subject.

use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome};
use radar_core::ui::icon::Icon;
use radar_core::ui;
use radar_core::view::ActionGroup;

use super::{Kind, Page, fetch, find_mr, mr_key};
use crate::GitlabPlugin;
use crate::model::{MergeSettings, Mr};
use crate::mr_actions::{self, ActMsg, Acting};

/// The repo a page is about, if it is a repo's or a merge request's.
fn repo_of(kind: &Kind) -> Option<&str> {
    match kind {
        Kind::Repo(path) => Some(path),
        Kind::Mr { project, .. } => Some(project),
        Kind::Tenancy(_) | Kind::Job { .. } => None,
    }
}

/// The page's own rows of its actions menu, in groups.
pub(super) fn bindings(p: &GitlabPlugin, page: &Page) -> Vec<crate::Row> {
    if repo_of(&page.kind).is_none() {
        return vec![];
    }
    let mut b = Vec::new();
    let mr = match &page.kind {
        Kind::Mr { project, iid } => find_mr(p, project, *iid),
        _ => None,
    };
    if let Some(mr) = mr {
        let busy = |what: &str| {
            p.acting
                .get(&mr_key(&mr.project_path, mr.iid))
                .is_some_and(|a| a.state == Acting::Running && a.what == what)
        };
        // The page is the merge request's: the label is the action alone, and
        // what stands in its way is a fact at the row's right edge.

        const WAITING: &str = "asked, waiting for GitLab";
        let mut rebase = Binding::new("u", "rebase…").edits().group(ActionGroup::Change);
        if busy("rebase") {
            rebase = Binding::new("u", "rebase").edits().group(ActionGroup::Change).hint(WAITING);
        }
        b.push(crate::act("gitlab.mr.rebase", rebase));
        let mut merge = Binding::new("m", "merge…").edits().group(ActionGroup::Change);
        if busy("merge") {
            merge = Binding::new("m", "merge").edits().group(ActionGroup::Change).hint(WAITING);
        } else if let Some(why) = mr_actions::blocker(mr) {
            merge = Binding::new("m", "merge").edits().group(ActionGroup::Change).hint(why);
        }
        b.push(crate::act("gitlab.mr.merge", merge));
        b.extend(super::review::bindings(p, page));
    }
    b
}

/// A page key of these actions; `None` when it is not one.
pub(super) fn key(p: &mut GitlabPlugin, c: char, kind: &Kind) -> Option<Outcome> {
    let project = repo_of(kind)?.to_string();
    let say = |m: String| Some(Outcome::Handled(vec![Effect::Notify(m)]));
    let fx = match c {
        'u' | 'm' => {
            let Kind::Mr { iid, .. } = kind else {
                return None;
            };
            let Some(mr) = find_mr(p, &project, *iid).cloned() else {
                return say(format!("!{iid} is not in the last sync"));
            };
            match c {
                'u' => ask_rebase(p, &mr),
                _ => ask_merge(p, &mr),
            }
        }
        _ => return None,
    };
    Some(Outcome::Handled(fx))
}

/// The links on the page (`Page::links`) as a menu: choosing one shows it,
/// which opens a ticket's, merge request's or thread's page here, else the
/// link the radar way.
pub(crate) fn links_menu(p: &GitlabPlugin, iid: u64) -> Vec<Effect> {
    let Some(page) = p.pages.last() else {
        return vec![];
    };
    let links = page.links.borrow();
    if links.is_empty() {
        return vec![Effect::Notify(format!(
            "nothing on !{iid}'s page links anywhere"
        ))];
    }
    let items = links
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let what = s.kind.replace('-', " ");
            let note = match s.url.as_deref() {
                Some(u) if s.kind == "link" => ui::fit(u, 60),
                Some(u) => format!("{what} · {}", ui::fit(u, 50)),
                None => what,
            };
            MenuItem::pick(ui::fit(&s.label, 48), note, i.to_string())
        })
        .collect();
    vec![Effect::Menu(
        Menu::new(
            format!("{LINK_MENU}:{iid}"),
            "follow a link on this page",
            items,
        )
        .icon(Icon::Link),
    )]
}

/// The question before a rebase, the page's `u` and the tree row's: the
/// kit's plain write confirm (it edits: the host adds the mode's note).
pub(crate) fn ask_rebase(p: &GitlabPlugin, mr: &Mr) -> Vec<Effect> {
    vec![
        mr_actions::journal(p, "gitlab.rebase.request", &mr.project_path, mr.iid),
        Effect::Menu(
            Menu::write_plain(
                format!("act-rebase:{}", mr_key(&mr.project_path, mr.iid)),
                format!("rebase {} onto {}?", mr.source_branch, mr.target_branch),
                vec![format!(
                    "GitLab rewrites the branch on top of {}",
                    mr.target_branch
                )],
                "rebase it",
                "leave it as it is",
            )
            .icon(Icon::Rebase),
        ),
    ]
}

/// The question before a merge, or why it cannot be asked yet.
pub(crate) fn ask_merge(p: &GitlabPlugin, mr: &Mr) -> Vec<Effect> {
    if let Some(why) = mr_actions::blocker(mr) {
        return vec![
            mr_actions::journal(p, "gitlab.merge", &mr.project_path, mr.iid),
            Effect::Report(radar_core::journal::Outcome::Blocked),
            Effect::Notify(format!("cannot merge !{} yet: {why}", mr.iid)),
        ];
    }
    let settings = super::changes::detail(p, &mr.project_path, mr.iid).map(|d| d.merge);
    let key = mr_key(&mr.project_path, mr.iid);
    vec![
        mr_actions::journal(p, "gitlab.merge", &mr.project_path, mr.iid),
        Effect::Menu(merge_menu(&key, mr, settings)),
    ]
}

/// The merge's question, with what will happen: where it goes, whether the
/// commits are squashed and the source branch deleted (as the merge
/// request and its repo are set; sent as shown).
pub(super) fn merge_menu(key: &str, mr: &Mr, settings: Option<MergeSettings>) -> Menu {
    let mut notes = vec![format!("{} → {}", mr.source_branch, mr.target_branch)];
    let value = match settings {
        Some(s) => {
            notes.push(if s.squash {
                "squash: yes, its commits become one".to_string()
            } else {
                "squash: no, its commits are kept".to_string()
            });
            notes.push(match (s.remove_source, s.remove_forced) {
                (true, true) => format!(
                    "{}: deleted after the merge (the repo says so)",
                    mr.source_branch
                ),
                (true, false) => format!("{}: deleted after the merge", mr.source_branch),
                (false, _) => format!("{}: kept", mr.source_branch),
            });
            format!("yes:{}:{}", u8::from(s.squash), u8::from(s.remove_source))
        }
        None => {
            notes.push("squash and the source branch: as GitLab is set for it".to_string());
            "yes".to_string()
        }
    };
    notes.push("GitLab checks it once more, then merges that commit".to_string());
    // The kit's plain write confirm (it edits: the host adds the mode's
    // note), as the rebase's: drawn as a dialog, its buttons `y`/`n`.
    let mut m = Menu::write_plain(
        format!("act-merge:{key}"),
        format!(
            "merge !{} into {}? It cannot be undone.",
            mr.iid, mr.target_branch
        ),
        notes,
        "merge it",
        "leave it open",
    )
    .icon(Icon::Merged);
    m.items[0].icon = Some(Icon::Branch);
    // `yes` carries what the question showed (squash, the source branch's
    // fate), so the merge sends exactly that: see `on_menu`.
    if let Some(yes) = m.items.iter_mut().find(|i| i.value == "yes") {
        yes.value = value;
    }
    m
}

/// The menu of a page's links; its value is the link's place in `Page::links`.
const LINK_MENU: &str = "mr-link";

/// A choice in one of the menus these actions open; `None` if not theirs.
pub(crate) fn on_menu(p: &mut GitlabPlugin, menu: &str, value: &str) -> Option<Vec<Effect>> {
    let (kind, arg) = menu.split_once(':')?;
    if kind == LINK_MENU {
        let i: usize = value.parse().ok()?;
        let s = p.pages.last()?.links.borrow().get(i).cloned()?;
        return Some(vec![Effect::Show(s)]);
    }
    if !matches!(kind, "act-rebase" | "act-merge") {
        return None;
    }
    if value != "yes" && !value.starts_with("yes:") {
        return Some(vec![Effect::Report(radar_core::journal::Outcome::Cancelled)]);
    }
    let (project, iid) = arg.rsplit_once('!')?;
    let iid: u64 = iid.parse().ok()?;
    Some(match kind {
        "act-rebase" => mr_actions::start(p, project, iid, "rebase"),
        _ => {
            // What the question showed: squash and delete the source branch.
            let mut parts = value.split(':').skip(1);
            let shown = match (parts.next(), parts.next()) {
                (Some(s), Some(r)) => Some(MergeSettings {
                    squash: s == "1",
                    remove_source: r == "1",
                    remove_forced: false,
                }),
                _ => None,
            };
            p.merge_as.insert(mr_key(project, iid), shown);
            mr_actions::start(p, project, iid, "merge")
        }
    })
}

/// A rebase, merge or write finished: the page and the tree are refreshed,
/// to show what GitLab made of it.
pub(crate) fn on_msg(p: &mut GitlabPlugin, msg: Msg) -> Vec<Effect> {
    // Always keep the original result, notice and refused text. Only the
    // automatic follow-up reads depend on the connection still being current.
    let (key, connection, mut fx) = match msg.downcast::<crate::write::WroteMsg>() {
        Ok(m) => {
            if !p.recovery.owns(&m) {
                return crate::write::completion_skipped(&m);
            }
            let key = m.change.key();
            (key, m.connection, crate::write::on_wrote(p, *m))
        }
        Err(msg) => {
            let Ok(done) = msg.downcast::<ActMsg>() else {
                return vec![];
            };
            (
                done.key.clone(),
                done.connection,
                mr_actions::on_msg(p, *done),
            )
        }
    };
    if !p.is_current_connection(connection) {
        fx.push(crate::write_context::skipped());
        return fx;
    }
    if let Some((project, iid)) = key.rsplit_once('!')
        && let Ok(iid) = iid.parse()
    {
        let kind = Kind::Mr {
            project: project.to_string(),
            iid,
        };
        fx.extend(fetch(p, &kind, true));
    }
    fx.extend(p.refresh());
    fx
}
