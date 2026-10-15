//! Exact persisted source identities, without ticket/project/site-alias inference.

use radar_core::journal::SubjectKey;
use radar_core::{Plugin, Subject};

use crate::{JiraPlugin, config, join, subject_scope};

pub(crate) fn from_journal(p: &JiraPlugin, key: &SubjectKey) -> Option<Subject> {
    if key.provider != "jira"
        || key.kind != join::TICKET
        || !config::is_ticket_key(&key.id)
        || SubjectKey::new(&key.provider, &key.instance, &key.kind, &key.id).ok()? != *key
    {
        return None;
    }
    let url = format!("{}/browse/{}", key.instance.trim_end_matches('/'), key.id);
    let subject = Subject::new(join::TICKET, &key.id, &key.id).with_url(url);
    // URL-only resolution requires one configured origin. Do not select an
    // account alias when multiple aliases serve that same persisted instance.
    let site = subject_scope::resolve(p, &subject)?;
    let restored = subject.with("site", &site.name);
    (p.journal_subject(&restored)?.key == *key).then_some(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_navigation_roundtrips_exact_site_even_for_duplicate_ticket_keys() {
        let p = crate::demo::sample();
        for alias in ["acme", "beta"] {
            let original = join::ticket(&p, alias, "ABC-12");
            let key = p.journal_subject(&original).unwrap().key;
            let restored = p.subject_from_journal(&key).unwrap();
            assert_eq!(restored.field("site"), Some(alias));
            assert_eq!(p.journal_subject(&restored).unwrap().key, key);
            assert_eq!(restored.label, "ABC-12");
            assert!(restored.field("title").is_none());
        }
    }

    #[test]
    fn source_navigation_rejects_wrong_scope_unsafe_and_unavailable_sites() {
        let mut p = crate::demo::sample();
        let key = p
            .journal_subject(&join::ticket(&p, "acme", "ABC-12"))
            .unwrap()
            .key;
        for (field, value) in [
            ("provider", "other-provider"),
            ("kind", "repository"),
            ("instance", "https://foreign.example/"),
            ("instance", "https://user:password@acme.atlassian.net/"),
            ("instance", "https://acme.atlassian.net/../"),
            ("instance", "https://acme.atlassian.net/\n"),
            ("id", "ABC-12/../ABC-13"),
            ("id", "ABC-12?token=secret"),
            ("id", "ABC-12\n"),
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
        let mut duplicate = p.cfg.site("acme").unwrap().clone();
        duplicate.name = "same-origin-another-account".into();
        p.cfg.sites.push(duplicate);
        assert!(p.subject_from_journal(&key).is_none());
        p.cfg.sites.clear();
        assert!(p.subject_from_journal(&key).is_none());
    }
}
