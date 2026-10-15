//! What the dock shows at once and refreshes in the background, and the session kept ready.
//!
//! The pickers (`alt-m`, `alt-a`, `alt-k`) and the `/` menu show what was read last, from this
//! cache, and a read in the background replaces it when the answer differs. The cache is saved
//! with the part's other state, so the first open after a restart is instant too. Keys carry
//! what the answer depends on: the provider, the working folder and, for agents and skills, the
//! setup (skill, plugin and instruction folders, project configuration). Time is the host's
//! clock, passed in, so nothing here reads the wall clock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapter::{Catalog, ModelInfo, SessionSetup, SlashCommand};
use crate::event::{Event, RadarSessionId, ProviderSessionId};

/// A model list read less than this many seconds ago is not read again.
pub const MODELS_FRESH: i64 = 3600;
/// An agent and skill catalog read less than this many seconds ago is not read again when a
/// picker opens (it is shown at once either way).
pub const CATALOG_FRESH: i64 = 600;
/// Entries kept per kind: the most recently read.
const KEEP: usize = 24;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stamped<T> {
    /// When it was read (host seconds).
    pub at: i64,
    pub value: T,
}

impl<T> Stamped<T> {
    /// Read within the last `ttl` seconds (a clock that went backwards counts as stale).
    pub fn fresh(&self, now: i64, ttl: i64) -> bool {
        self.at <= now && now - self.at < ttl
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// By provider.
    #[serde(default)]
    models: BTreeMap<String, Stamped<Vec<ModelInfo>>>,
    /// By [`catalog_key`].
    #[serde(default)]
    catalogs: BTreeMap<String, Stamped<Catalog>>,
    /// By [`commands_key`].
    #[serde(default)]
    commands: BTreeMap<String, Stamped<Vec<SlashCommand>>>,
}

/// The oldest entries beyond [`KEEP`] go.
fn trim<T>(map: &mut BTreeMap<String, Stamped<T>>) {
    while map.len() > KEEP {
        let Some(oldest) = map.iter().min_by_key(|(_, v)| v.at).map(|(k, _)| k.clone()) else { return };
        map.remove(&oldest);
    }
}

/// FNV-1a: stable across runs, which `DefaultHasher` does not promise.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// What a catalog read depends on: the provider, the folder and the parts of the setup that
/// change what the CLI finds. The agent chosen, the skills asked for and the review flag do not.
pub fn catalog_key(provider: &str, cwd: &Path, setup: &SessionSetup) -> String {
    let mut s = setup.clone();
    s.agent = None;
    s.skills.clear();
    s.review = false;
    let json = serde_json::to_string(&s).unwrap_or_default();
    format!("{provider}|{}|{:016x}", cwd.display(), fnv(&json))
}

/// Commands depend on the provider and the folder (a project's own commands and skills).
pub fn commands_key(provider: &str, cwd: &str) -> String {
    format!("{provider}|{cwd}")
}

impl Cache {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.catalogs.is_empty() && self.commands.is_empty()
    }

    pub fn models(&self, provider: &str) -> Option<&Stamped<Vec<ModelInfo>>> {
        self.models.get(provider)
    }

    /// Every provider's models, for seeding the picker at start.
    pub fn all_models(&self) -> impl Iterator<Item = (&String, &Vec<ModelInfo>)> {
        self.models.iter().map(|(p, m)| (p, &m.value))
    }

    pub fn set_models(&mut self, provider: &str, models: Vec<ModelInfo>, now: i64) {
        self.models.insert(provider.to_string(), Stamped { at: now, value: models });
    }

    pub fn catalog(&self, key: &str) -> Option<&Stamped<Catalog>> {
        self.catalogs.get(key)
    }

    pub fn set_catalog(&mut self, key: &str, catalog: Catalog, now: i64) {
        self.catalogs.insert(key.to_string(), Stamped { at: now, value: catalog });
        trim(&mut self.catalogs);
    }

    pub fn commands(&self, key: &str) -> Option<&[SlashCommand]> {
        self.commands.get(key).map(|c| c.value.as_slice())
    }

    /// Remember what a session of this provider and folder offers after `/`. `true` when it
    /// differs from what was kept (so it is worth saving).
    pub fn set_commands(&mut self, key: &str, commands: Vec<SlashCommand>, now: i64) -> bool {
        if commands.is_empty() {
            return false;
        }
        let changed = self.commands.get(key).is_none_or(|c| c.value != commands);
        if changed {
            self.commands.insert(key.to_string(), Stamped { at: now, value: commands });
            trim(&mut self.commands);
        }
        changed
    }
}

/// A session created ahead of the next new tab and not shown yet. Its events wait here and
/// become the new tab's history when it is taken; nothing about it reaches the store, the dock
/// or the saved state before that.
pub struct Spare {
    pub radar: RadarSessionId,
    pub provider: String,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub setup: SessionSetup,
    /// The provider's id for it, once the adapter has created it.
    pub psid: Option<ProviderSessionId>,
    pub events: Vec<Event>,
    pub commands: Option<Vec<SlashCommand>>,
}

impl Spare {
    /// Whether a new tab with these inputs is exactly this session.
    pub fn fits(&self, provider: &str, cwd: &Path, model: &Option<String>, setup: &SessionSetup) -> bool {
        self.provider == provider && self.cwd == cwd && &self.model == model && &self.setup == setup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> SlashCommand {
        SlashCommand::new(name, "")
    }

    #[test]
    fn a_catalog_key_follows_the_folder_and_the_folders_of_the_setup_but_not_the_agent_chosen() {
        let cwd = Path::new("/work/a");
        let base = SessionSetup::default();
        let key = catalog_key("claude", cwd, &base);
        assert_eq!(key, catalog_key("claude", cwd, &base), "stable");
        assert_ne!(key, catalog_key("codex", cwd, &base));
        assert_ne!(key, catalog_key("claude", Path::new("/work/b"), &base));
        let with_skills = SessionSetup { skill_dirs: vec!["/skills".into()], ..base.clone() };
        assert_ne!(key, catalog_key("claude", cwd, &with_skills));
        let asked = SessionSetup { agent: Some("reviewer".into()), skills: vec!["notes".into()], review: true, ..base };
        assert_eq!(key, catalog_key("claude", cwd, &asked), "what is chosen from the catalog does not change the catalog");
    }

    #[test]
    fn freshness_counts_seconds_on_the_hosts_clock_and_a_clock_that_went_back_is_stale() {
        let s = Stamped { at: 100, value: () };
        assert!(s.fresh(100, 600));
        assert!(s.fresh(699, 600));
        assert!(!s.fresh(700, 600));
        assert!(!s.fresh(99, 600));
    }

    #[test]
    fn the_cache_keeps_the_most_recent_entries_and_survives_a_round_trip() {
        let mut c = Cache::default();
        for n in 0..(KEEP + 5) {
            c.set_catalog(&format!("k{n:03}"), Catalog::default(), n as i64);
        }
        assert!(c.catalog("k000").is_none(), "the oldest went");
        assert!(c.catalog(&format!("k{:03}", KEEP + 4)).is_some());
        c.set_models("fake", vec![ModelInfo { id: "m".into(), name: "M".into() }], 7);
        assert!(c.set_commands("fake|/w", vec![entry("plan")], 8));
        assert!(!c.set_commands("fake|/w", vec![entry("plan")], 9), "the same list is not news");
        assert!(!c.set_commands("fake|/w", vec![], 9), "an empty list never replaces a real one");
        let back: Cache = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.models("fake").unwrap().at, 7);
        assert_eq!(back.commands("fake|/w").unwrap()[0].name, "plan");
    }

    #[test]
    fn a_spare_fits_only_the_exact_inputs_it_was_made_with() {
        let spare = Spare {
            radar: RadarSessionId::new("s"),
            provider: "fake".into(),
            cwd: "/w".into(),
            model: None,
            setup: SessionSetup::default(),
            psid: None,
            events: vec![],
            commands: None,
        };
        assert!(spare.fits("fake", Path::new("/w"), &None, &SessionSetup::default()));
        assert!(!spare.fits("other", Path::new("/w"), &None, &SessionSetup::default()));
        assert!(!spare.fits("fake", Path::new("/x"), &None, &SessionSetup::default()));
        assert!(!spare.fits("fake", Path::new("/w"), &Some("m".into()), &SessionSetup::default()));
        assert!(!spare.fits("fake", Path::new("/w"), &None, &SessionSetup { agent: Some("a".into()), ..Default::default() }));
    }
}
