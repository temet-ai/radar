//! Cached navigation candidates. No discovery, client construction or reads.

use std::collections::BTreeSet;

use radar_core::{Subject, palette::PaletteTarget, ui::icon::Icon};

use crate::{GitlabPlugin, join};

pub(crate) fn targets(p: &GitlabPlugin) -> Vec<PaletteTarget> {
    let Ok(origin) = radar_core::journal::SafeUrl::parse(&p.cfg.url) else {
        return vec![];
    };
    let scope = origin.as_str().trim_end_matches('/');
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |subject: Subject, words: Vec<String>, icon| {
        if join::Target::consistent(&p.cfg.url, &subject).is_some()
            && seen.insert((subject.kind.clone(), subject.id.clone()))
        {
            let mut target = PaletteTarget::scoped(subject, scope);
            target.words = words;
            target.icon = Some(icon);
            out.push(target);
        }
    };
    for data in p.data.values() {
        for project in &data.projects {
            let mut subject = join::repo(p, &project.full_path);
            subject.label.clone_from(&project.full_path);
            add(
                subject,
                vec![
                    project.name.clone(),
                    project.full_path.clone(),
                    "repository".into(),
                ],
                Icon::Repo,
            );
        }
    }
    // Prefer the page's freshly held copy when it also appears in a list.
    for mr in p
        .mr_pages
        .iter()
        .filter_map(|(key, fetched)| {
            let (mr, _) = fetched.value.as_ref()?;
            (key == &crate::pages::mr_key(&mr.project_path, mr.iid)).then_some(mr)
        })
        .chain(p.data.values().flat_map(|data| &data.mrs))
    {
        let mut subject = join::of_mr(mr);
        subject.label = format!("{} · {}", subject.label, mr.title);
        add(
            subject,
            vec![
                mr.title.clone(),
                mr.source_branch.clone(),
                mr.project_path.clone(),
                "merge request".into(),
            ],
            Icon::Change,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GroupData, model::Project, test_support::MrBuilder};
    use radar_core::Plugin;

    #[test]
    fn cached_repositories_and_merge_requests_keep_show_identity_once() {
        let mut p = GitlabPlugin::default();
        p.cfg.url = "https://gitlab.example.test".into();
        let mr = MrBuilder::default().build();
        let data = GroupData {
            projects: vec![Project {
                name: "City Watch".into(),
                full_path: "ankh-morpork/watch".into(),
                ..Default::default()
            }],
            mrs: vec![mr.clone()],
            ..Default::default()
        };
        p.data.insert("watch".into(), data.clone());
        p.data.insert("overlap".into(), data);
        p.mr_pages.insert(
            crate::pages::mr_key(&mr.project_path, mr.iid),
            crate::pages::Fetched {
                value: Some((mr.clone(), Default::default())),
                ..Default::default()
            },
        );
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].subject, join::repo(&p, "ankh-morpork/watch"));
        assert_eq!(targets[1].subject, join::of_mr(&mr));
        assert!(targets[1].words.contains(&"Repair the Watch bell".into()));
        for target in targets {
            let route = join::Target::consistent(&p.cfg.url, &target.subject).unwrap();
            assert_eq!(route.project, "ankh-morpork/watch");
            assert!(target.subject.field("project").is_some());
            if let Some(field) = route.number_field() {
                assert!(target.subject.field(field).is_some());
            }
            let radar_core::Showing::Page { screen, effects } = p.show(&target.subject) else {
                panic!("cached target opens")
            };
            assert_eq!(screen, crate::SCREEN.id);
            assert_eq!(
                p.subject(screen, radar_core::Pane::Middle).unwrap().id,
                target.subject.id
            );
            // Provider reads remain unevaluated effect envelopes.
            drop(effects);
        }
    }

    #[test]
    fn empty_or_foreign_cached_items_never_become_palette_destinations() {
        let mut p = GitlabPlugin::default();
        p.cfg.url = "https://gitlab.example.test".into();
        assert!(p.palette_targets().is_empty());
        let mut mr = MrBuilder::default().build();
        mr.web_url = "https://foreign.example.test/ankh-morpork/watch/-/merge_requests/12".into();
        p.data.insert(
            "watch".into(),
            GroupData {
                mrs: vec![mr],
                ..Default::default()
            },
        );
        assert!(p.palette_targets().is_empty());
    }
    #[test]
    fn palette_identity_survives_metadata_changes_but_not_server_changes() {
        let mut p = GitlabPlugin::default();
        p.cfg.url = "https://gitlab.example.test".into();
        p.data.insert(
            "watch".into(),
            GroupData {
                mrs: vec![MrBuilder::default().build()],
                ..Default::default()
            },
        );
        let before = p.palette_targets().remove(0).identity;
        let mr = &mut p.data.get_mut("watch").unwrap().mrs[0];
        mr.title = "Inspect the Watch bell".into();
        mr.head_sha = "b".repeat(40);
        mr.state = crate::model::MrState::Merged;
        assert_eq!(p.palette_targets()[0].identity, before);
        p.cfg.url = "https://vimes:synthetic-password@gitlab.example.test/?token=synthetic".into();
        let scoped = &p.palette_targets()[0].identity;
        assert_eq!(scoped, &before);
        assert!(!scoped.contains("synthetic"));
        p.cfg.url = "https://lancre.example.test".into();
        p.data.get_mut("watch").unwrap().mrs[0].web_url =
            "https://lancre.example.test/ankh-morpork/watch/-/merge_requests/12".into();
        assert_ne!(p.palette_targets()[0].identity, before);
    }
}
