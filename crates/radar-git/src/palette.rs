//! Cached checkout/worktree destinations, validated by the existing exact path
//! routing gate. Enumerating these never expands paths or inspects the disk.

use std::collections::BTreeMap;

use radar_core::{journal::SubjectKey, palette::PaletteTarget, ui::icon::Icon};

use crate::{GitPlugin, source_navigation};

pub(crate) fn targets(p: &GitPlugin) -> Vec<PaletteTarget> {
    let mut targets = BTreeMap::new();
    let mut add = |dir: &str, project: &str, branch: &str| {
        let Ok(key) = SubjectKey::new("local", dir, "checkout", dir) else {
            return;
        };
        let Some(subject) = source_navigation::from_journal(p, &key) else {
            return;
        };
        targets.entry(dir.to_string()).or_insert_with(|| {
            let mut target = PaletteTarget::new(subject);
            target.words = vec![
                project.into(),
                branch.into(),
                "checkout".into(),
                "worktree".into(),
            ];
            target.icon = Some(Icon::Local);
            target
        });
    };
    for checkout in p
        .local
        .index
        .iter()
        .flat_map(|index| &index.checkouts)
        .chain(&p.local.linked)
    {
        add(&checkout.dir, &checkout.project, &checkout.branch);
        if checkout.main.is_empty()
            && let Some(detail) = p.branches.details.get(&checkout.dir)
        {
            for tree in &detail.trees {
                add(&tree.dir, &checkout.project, &tree.branch);
            }
        }
    }
    targets.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        local::{Checkout, Index},
        screen::{Detail, Tree},
    };
    use radar_core::{Plugin, Showing};

    #[test]
    fn cached_clone_and_worktree_targets_select_exact_folders_without_effects() {
        let mut p = GitPlugin::default();
        let checkout = Checkout {
            dir: "/work/ankh-morpork/watch".into(),
            project: "ankh-morpork/watch".into(),
            branch: "main".into(),
            ..Default::default()
        };
        p.local.index = Some(Index {
            checkouts: vec![checkout.clone()],
            ..Default::default()
        });
        p.local.linked.push(checkout.clone());
        p.branches.details.insert(
            checkout.dir.clone(),
            Detail {
                dir: checkout.dir.clone(),
                trees: vec![
                    Tree {
                        dir: checkout.dir.clone(),
                        branch: "main".into(),
                        ..Default::default()
                    },
                    Tree {
                        dir: "/work/ankh-morpork/watch-bell".into(),
                        branch: "watch-bell".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        );
        let targets = p.palette_targets();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].subject.id, checkout.dir);
        assert_eq!(targets[1].subject.id, "/work/ankh-morpork/watch-bell");
        assert!(targets[1].words.contains(&"watch-bell".into()));
        let identities: Vec<_> = targets
            .iter()
            .map(|target| target.identity.clone())
            .collect();
        p.local.index.as_mut().unwrap().checkouts[0].branch = "patrol".into();
        p.branches.details.get_mut(&checkout.dir).unwrap().trees[1].branch =
            "bell-inspection".into();
        assert_eq!(
            p.palette_targets()
                .into_iter()
                .map(|target| target.identity)
                .collect::<Vec<_>>(),
            identities
        );
        // Navigation below must not write the newly cached state either.
        let before = p.snapshot();
        for target in targets {
            let Showing::Page { screen, effects } = p.show(&target.subject) else {
                panic!("cached folder opens")
            };
            assert!(matches!(
                effects.as_slice(),
                [radar_core::Effect::Focus(radar_core::Pane::Middle)]
            ));
            assert_eq!(
                p.subject(screen, radar_core::Pane::Middle).unwrap().id,
                target.subject.id
            );
        }
        assert_eq!(p.snapshot(), before);
    }

    #[test]
    fn unresolved_folders_and_orphaned_worktrees_are_not_targets() {
        let mut p = GitPlugin::default();
        assert!(p.palette_targets().is_empty());
        p.local.index = Some(Index {
            checkouts: vec![
                Checkout {
                    dir: "~/watch".into(),
                    ..Default::default()
                },
                Checkout {
                    dir: "/work/watch/../witches".into(),
                    ..Default::default()
                },
                Checkout {
                    dir: "/work/watch-bell".into(),
                    main: "/work/watch".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        assert!(p.palette_targets().is_empty());
    }
}
