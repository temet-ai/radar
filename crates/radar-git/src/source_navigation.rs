//! Exact local-folder identities; no path expansion, checkout scan or name match.

use std::path::Path;

use radar_core::journal::SubjectKey;
use radar_core::{Plugin, Showing, Subject};

use crate::{GitPlugin, local, screen};

pub(crate) fn from_journal(p: &GitPlugin, key: &SubjectKey) -> Option<Subject> {
    if key.provider != "local"
        || key.kind != "checkout"
        || key.instance != key.id
        || SubjectKey::new(&key.provider, &key.instance, &key.kind, &key.id).ok()? != *key
        || !Path::new(&key.id).is_absolute()
        || (!cfg!(windows) && key.id.contains('\\'))
        || key
            .id
            .split(['/', '\\'])
            .any(|part| matches!(part, "." | ".."))
        || local::journal_path(&key.id)?.to_str()? != key.id
    {
        return None;
    }
    // An old path is not authority to discover or open a new folder. It must
    // still be a known checkout, identified by the complete local path.
    screen::checkout_route(p, &key.id)?;
    let restored = Subject::new("checkout", &key.id, &key.id);
    (p.journal_subject(&restored)?.key == *key).then_some(restored)
}

pub(crate) fn show(p: &mut GitPlugin, subject: &Subject) -> Showing {
    let Some(key) = p.journal_subject(subject).map(|snapshot| snapshot.key) else {
        return Showing::Cannot;
    };
    // The caller's path itself must be canonical; a relative or repaired path
    // must not reach a known folder by being normalized by journal_subject.
    if key.id != subject.id || subject.url.is_some() || from_journal(p, &key).is_none() {
        return Showing::Cannot;
    }
    screen::show_checkout(p, &key.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_navigation_preserves_a_known_native_absolute_folder() {
        let mut p = crate::demo::branches();
        let dir = std::path::absolute("work-source-fixture")
            .unwrap()
            .display()
            .to_string();
        p.local.index.as_mut().unwrap().checkouts[0].dir = dir.clone();
        let source = Subject::new("checkout", &dir, "local folder");
        let key = p.journal_subject(&source).unwrap().key;
        assert_eq!(key.id, dir);
        assert_eq!(p.subject_from_journal(&key).unwrap().id, dir);
        assert!(matches!(p.show(&source), Showing::Page { .. }));
    }

    #[test]
    fn source_navigation_roundtrips_known_clones_and_worktrees() {
        let p = crate::demo::branches();
        for dir in ["/work/acme/api", "/work/acme/api-hotfix"] {
            let key = SubjectKey::new("local", dir, "checkout", dir).unwrap();
            let restored = p.subject_from_journal(&key).unwrap();
            assert_eq!(restored.id, dir);
            assert!(restored.fields.is_empty());
            assert_eq!(p.journal_subject(&restored).unwrap().key, key);
        }
    }

    #[test]
    fn source_navigation_rejects_other_scope_unknown_and_repaired_paths() {
        let mut p = crate::demo::branches();
        let key = SubjectKey::new("local", "/work/acme/api", "checkout", "/work/acme/api").unwrap();
        let mut bad = key.clone();
        bad.provider = "git".into();
        assert!(p.subject_from_journal(&bad).is_none());
        bad = key.clone();
        bad.kind = "repository".into();
        assert!(p.subject_from_journal(&bad).is_none());
        bad = key.clone();
        bad.instance = "/other/machine/api".into();
        assert!(p.subject_from_journal(&bad).is_none());
        for path in [
            "/work/acme/unknown",
            "~/work/acme/api",
            "work/acme/api",
            "/work/acme/../acme/api",
            "/work/acme/./api",
            "/work//acme/api",
            "/work/acme/api\n",
            "/work/acme/api?token=secret",
            "/work/acme\\api",
        ] {
            bad = key.clone();
            bad.instance = path.into();
            bad.id = path.into();
            assert!(p.subject_from_journal(&bad).is_none(), "{path:?}");
        }
        p.local.index = None;
        assert!(p.subject_from_journal(&key).is_none());
    }

    #[test]
    fn source_navigation_selects_only_the_exact_existing_folder_without_io_or_writes() {
        let mut p = crate::demo::branches();
        let before = p.snapshot();
        for dir in ["/work/acme/api", "/work/acme/api-hotfix"] {
            let subject = Subject::new("checkout", dir, "not routing identity");
            let Showing::Page { screen, effects } = p.show(&subject) else {
                panic!("known folder has a native view")
            };
            assert_eq!(screen, screen::SCREEN.id);
            assert!(matches!(
                effects.as_slice(),
                [radar_core::Effect::Focus(radar_core::Pane::Middle)]
            ));
            assert_eq!(p.subject(screen, radar_core::Pane::Middle).unwrap().id, dir);
            assert_eq!(p.snapshot(), before, "no folder selection or cache write");
        }
        let state = format!("{p:?}");
        for dir in [
            "/work/acme/unknown",
            "/work/acme/../acme/api",
            "work/acme/api",
        ] {
            assert!(matches!(
                p.show(&Subject::new("checkout", dir, "x")),
                Showing::Cannot
            ));
            assert_eq!(format!("{p:?}"), state);
        }
    }
}
