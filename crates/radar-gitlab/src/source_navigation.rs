//! Invert only exact journal identities through the owner's routing validator.

use radar_core::journal::{SafeUrl, SubjectKey};
use radar_core::{Plugin, Subject};

use crate::{GitlabPlugin, join};

pub(crate) fn from_journal(p: &GitlabPlugin, key: &SubjectKey) -> Option<Subject> {
    if key.provider != "gitlab"
        || p.cfg.sources.is_empty()
        || SubjectKey::new(&key.provider, &key.instance, &key.kind, &key.id).ok()? != *key
        || SafeUrl::parse(&p.cfg.url).ok()?.as_str() != key.instance
    {
        return None;
    }
    let root = key.instance.trim_end_matches('/');
    let (id, url) = match key.kind.as_str() {
        join::REPO => (key.id.clone(), format!("{root}/{}", key.id)),
        join::MERGE_REQUEST | join::PIPELINE | join::JOB => {
            let (project, number) = key.id.rsplit_once(if key.kind == join::MERGE_REQUEST {
                '!'
            } else {
                '/'
            })?;
            let parsed: u64 = number.parse().ok()?;
            if parsed == 0 || parsed.to_string() != number {
                return None;
            }
            let (marker, route) = match key.kind.as_str() {
                join::MERGE_REQUEST => ('!', "merge_requests"),
                join::PIPELINE => ('#', "pipelines"),
                _ => ('#', "jobs"),
            };
            (
                format!("{project}{marker}{number}"),
                format!("{root}/{project}/-/{route}/{number}"),
            )
        }
        _ => return None,
    };
    let target = join::Target::from_url(&p.cfg.url, &url)?;
    if target.kind != key.kind {
        return None;
    }
    let mut restored = Subject::new(&key.kind, id, &key.id)
        .with_url(url)
        .with("project", &target.project);
    if let Some(field) = target.number_field() {
        restored = restored.with(field, target.number?.to_string());
    }
    join::Target::consistent(&p.cfg.url, &restored)?;
    (p.journal_subject(&restored)?.key == *key).then_some(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_navigation_roundtrips_each_owned_kind_and_installation_prefix() {
        for instance in ["https://gitlab.com", "https://code.example/gitlab"] {
            let mut p = crate::demo::sample();
            p.cfg.url = instance.into();
            for (kind, id) in [
                (join::REPO, "acme/team/api"),
                (join::MERGE_REQUEST, "acme/team/api!17"),
                (join::PIPELINE, "acme/team/api/42"),
                (join::JOB, "acme/team/api/51"),
            ] {
                let key = SubjectKey::new("gitlab", instance, kind, id).unwrap();
                let restored = p.subject_from_journal(&key).unwrap();
                assert_eq!(restored.field("project"), Some("acme/team/api"));
                assert_eq!(p.journal_subject(&restored).unwrap().key, key);
                assert!(restored.field("title").is_none());
            }
        }
    }

    #[test]
    fn source_navigation_rejects_wrong_scope_unsafe_and_unavailable_config() {
        let mut p = crate::demo::sample();
        let key =
            SubjectKey::new("gitlab", &p.cfg.url, join::MERGE_REQUEST, "acme/api!17").unwrap();
        for (field, value) in [
            ("provider", "other-provider"),
            ("kind", "pull-request"),
            ("instance", "https://other.example/"),
            ("instance", "https://user:password@gitlab.com/"),
            ("instance", "https://gitlab.com/../"),
            ("id", "acme/../api!17"),
            ("id", "acme/api!017"),
            ("id", "acme/api!0"),
            ("id", "acme/api!17\n"),
            ("id", "acme/api!17?token=secret"),
        ] {
            let mut bad = key.clone();
            match field {
                "provider" => bad.provider = value.into(),
                "kind" => bad.kind = value.into(),
                "instance" => bad.instance = value.into(),
                _ => bad.id = value.into(),
            }
            assert!(p.subject_from_journal(&bad).is_none(), "{field}: {value:?}");
        }
        p.cfg.sources.clear();
        assert!(p.subject_from_journal(&key).is_none());
    }
}
