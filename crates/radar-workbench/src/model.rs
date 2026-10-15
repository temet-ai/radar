//! A bounded projection of explicit claims. No name matching creates identity.
use radar_core::journal::work_map::{
    Basis, ChangeRecord, Handoff, HandoffRecord, ItemId, Mutation, TagEntry, TagId, TagTarget,
};

pub const TITLE_NAMESPACE: &str = "work.title";
pub const GROUP_NAMESPACE: &str = "work.group";

#[derive(Clone)]
pub struct Label {
    pub id: TagId,
    pub namespace: String,
    pub text: String,
    pub ambiguous: bool,
}

pub fn catalog_label(tag: &TagEntry) -> Label {
    let mut renames = tag
        .renames
        .iter()
        .map(|claim| claim.label.as_str())
        .collect::<Vec<_>>();
    renames.sort_unstable();
    renames.dedup();
    let ambiguous = tag.labels_truncated || renames.len() > 1;
    Label {
        id: tag.tag_id.clone(),
        namespace: GROUP_NAMESPACE.into(),
        text: if !ambiguous && renames.len() == 1 {
            renames[0]
        } else {
            tag.creation.label.as_str()
        }
        .into(),
        ambiguous,
    }
}

pub fn claims<'a>(pages: &[&'a Handoff]) -> Vec<&'a ChangeRecord> {
    let mut found = Vec::new();
    for page in pages {
        for record in &page.records {
            if let HandoffRecord::Map { change } = record
                && !found
                    .iter()
                    .any(|c: &&ChangeRecord| c.request.id == change.request.id)
            {
                found.push(change.as_ref());
            }
        }
    }
    found
}

pub fn labels_for(item: &ItemId, pages: &[&Handoff]) -> Vec<Label> {
    let records = claims(pages);
    let mut tags = Vec::new();
    for claim in &records {
        if claim.retracted || claim.request.provenance.basis != Basis::UserAsserted {
            continue;
        }
        let Mutation::AssignTag {
            tag_id,
            target: TagTarget::Item(target),
        } = &claim.request.mutation
        else {
            continue;
        };
        if target != item || tags.iter().any(|tag: &Label| tag.id == *tag_id) {
            continue;
        }
        let Some((namespace, original)) = records.iter().find_map(|c| {
            if c.retracted || c.request.provenance.basis != Basis::UserAsserted {
                return None;
            }
            match &c.request.mutation {
                Mutation::CreateTag {
                    tag_id: id,
                    namespace,
                    label,
                    ..
                } if id == tag_id => Some((namespace.as_str(), label.as_str())),
                _ => None,
            }
        }) else {
            continue;
        };
        let mut renames = records
            .iter()
            .filter_map(|c| {
                if c.retracted || c.request.provenance.basis != Basis::UserAsserted {
                    return None;
                }
                match &c.request.mutation {
                    Mutation::RenameTag { tag_id: id, label } if id == tag_id => {
                        Some(label.as_str())
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();
        renames.sort_unstable();
        renames.dedup();
        tags.push(Label {
            id: tag_id.clone(),
            namespace: namespace.into(),
            text: if renames.len() == 1 {
                renames[0]
            } else {
                original
            }
            .into(),
            ambiguous: renames.len() > 1,
        });
    }
    tags
}

pub fn title(item: &ItemId, pages: &[&Handoff]) -> String {
    let titles = labels_for(item, pages)
        .into_iter()
        .filter(|label| label.namespace == TITLE_NAMESPACE)
        .collect::<Vec<_>>();
    match titles.as_slice() {
        [label] if !label.ambiguous => label.text.clone(),
        [] => {
            let bindings = claims(pages)
                .into_iter()
                .filter_map(|claim| {
                    if claim.retracted {
                        return None;
                    }
                    match &claim.request.mutation {
                        Mutation::BindSubject {
                            item_id, subject, ..
                        } if item_id == item => Some(subject),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            match bindings.as_slice() {
                [source] => format!("{} · {}", source.kind, source.id),
                _ => format!("Item {}", &item.as_str()[item.as_str().len() - 8..]),
            }
        }
        _ => format!("Conflicting local titles · {}", &item.as_str()[..8]),
    }
}

pub fn basis(basis: Basis) -> &'static str {
    match basis {
        Basis::Observed => "observed",
        Basis::UserAsserted => "authored",
        Basis::Inferred => "inferred",
    }
}
