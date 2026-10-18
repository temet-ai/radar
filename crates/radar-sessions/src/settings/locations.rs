//! Repository-specific locations. Unset fields inherit independently.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::expand;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RepositoryLocations {
    /// Local checkout used to identify a repository when opening a plain dock session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviews_dir: Option<String>,
}

/// An instance-qualified repository wins over an unqualified project path.
pub fn repository_entry<'a, T>(entries: &'a BTreeMap<String, T>, instance: &str, repository: &str) -> Option<&'a T> {
    let repository = repository.trim_end_matches('/').trim_end_matches(".git");
    let scoped = format!("{}/{repository}", instance.trim_end_matches('/'));
    entries.get(&scoped).or_else(|| entries.get(repository))
}

/// A nested checkout wins over its enclosing checkout, without matching sibling names.
pub fn checkout_entry<'a>(entries: &'a BTreeMap<String, RepositoryLocations>, cwd: &Path) -> Option<&'a RepositoryLocations> {
    entries.values().filter_map(|entry| {
        let checkout = entry.checkout.as_deref().and_then(expand)?;
        cwd.starts_with(&checkout).then_some((checkout.components().count(), entry))
    }).max_by_key(|(depth, _)| *depth).map(|(_, entry)| entry)
}

pub fn review_dirs(entries: &BTreeMap<String, RepositoryLocations>) -> BTreeMap<String, PathBuf> {
    entries.iter().filter_map(|(repo, entry)| {
        Some((repo.clone(), expand(entry.reviews_dir.as_deref()?)?))
    }).collect()
}

impl super::AgentsSettings {
    pub(crate) fn repository_menu(&self) -> Vec<radar_core::Effect> {
        use radar_core::{Effect, Menu, MenuItem};
        let mut rows: Vec<_> = self.cfg.repositories.keys()
            .map(|repo| MenuItem::pick(repo, "review and working folders", repo)).collect();
        rows.push(MenuItem::new('a', "add a repository", "its project path or service URL", "@add"));
        vec![Effect::Menu(Menu::new("agents.repositories", "Repository locations", rows))]
    }

    pub(crate) fn repository_location_menu(&mut self, menu: &str, value: &str) -> Option<Vec<radar_core::Effect>> {
        use radar_core::{Effect, Menu, MenuItem};
        if menu == "agents.repositories" && value == "@add" {
            return Some(vec![Effect::Menu(Menu::typed("agents.repository_add", "Repository path or service URL", ""))]);
        }
        if menu == "agents.repository_add" {
            if value.is_empty() || !value.contains('/') || value.contains(char::is_whitespace) || super::looks_like_token(value) {
                return Some(vec![Effect::error(None, "give a repository path, such as ankh-morpork/watch")]);
            }
            self.cfg.repositories.entry(value.into()).or_default();
        }
        if matches!(menu, "agents.repositories" | "agents.repository_add") {
            let entry = self.cfg.repositories.get(value)?;
            let field = |key, label, field: &str, current: &Option<String>| MenuItem::new(
                key, label, current.as_deref().unwrap_or("inherit"), format!("{field}\t{value}"),
            );
            return Some(vec![Effect::Menu(Menu::new("agents.repository_fields", value, vec![
                field('c', "checkout folder", "checkout", &entry.checkout),
                field('w', "working folder", "work_dir", &entry.work_dir),
                field('r', "reviews folder", "reviews_dir", &entry.reviews_dir),
                MenuItem::new('d', "drop overrides", "use the global defaults; files stay where they are", format!("remove\t{value}")),
            ]))]);
        }
        if menu == "agents.repository_fields" {
            let (field, repo) = value.split_once('\t')?;
            if field == "remove" {
                self.cfg.repositories.remove(repo);
                return Some(self.repository_menu());
            }
            let entry = self.cfg.repositories.get(repo)?;
            let (label, value) = match field {
                "checkout" => ("Checkout folder (empty: no local matching)", &entry.checkout),
                "work_dir" => ("Working folder (empty: inherit)", &entry.work_dir),
                "reviews_dir" => ("Reviews folder (empty: inherit)", &entry.reviews_dir),
                _ => return None,
            };
            return Some(vec![Effect::Menu(Menu::typed(format!("agents.repository_path:{field}:{repo}"), label, value.as_deref().unwrap_or_default()))]);
        }
        let (field, repo) = menu.strip_prefix("agents.repository_path:")?.split_once(':')?;
        if !value.is_empty() && (expand(value).is_none() || super::looks_like_token(value)) {
            return Some(vec![Effect::error(None, "give the whole path (or ~/…); not saved")]);
        }
        let entry = self.cfg.repositories.get_mut(repo)?;
        let slot = match field { "checkout" => &mut entry.checkout, "work_dir" => &mut entry.work_dir, "reviews_dir" => &mut entry.reviews_dir, _ => return None };
        *slot = (!value.is_empty()).then(|| value.to_string());
        Some(vec![Effect::Notify(format!("saved {field} for {repo}; applies to new sessions and reviews"))])
    }
}
