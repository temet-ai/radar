//! Owner-local model fixtures and strict offline transport for hosted tests.
//!
//! Defaults describe one small Watch repository, not the richer demo/gallery world.
//! Each model build owns its data, with no I/O, clock or shared mutable state.
//! The separate hosted fixture scripts real production requests and parsing.

mod hosted;
pub use hosted::{ExpectedRequest, HostedFixture, Reply, Request, RequestRecord, hosted_plugin};

use crate::model::{Repository, RepositoryId};

#[derive(Clone)]
pub struct RepositoryBuilder {
    repository: Repository,
}

impl Default for RepositoryBuilder {
    fn default() -> Self {
        Self {
            repository: Repository {
                id: RepositoryId::parse("ankh-morpork/watch").unwrap(),
                description: None,
                default_branch: "main".into(),
                private: false,
                archived: false,
            },
        }
    }
}

impl RepositoryBuilder {
    pub fn id(mut self, id: RepositoryId) -> Self {
        self.repository.id = id;
        self
    }

    pub fn description(mut self, description: &str) -> Self {
        self.repository.description = Some(description.into());
        self
    }

    pub fn default_branch(mut self, branch: &str) -> Self {
        self.repository.default_branch = branch.into();
        self
    }

    pub fn private(mut self, private: bool) -> Self {
        self.repository.private = private;
        self
    }

    pub fn archived(mut self, archived: bool) -> Self {
        self.repository.archived = archived;
        self
    }

    pub fn build(self) -> Repository {
        self.repository
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_minimal_and_repeatable() {
        let repository = RepositoryBuilder::default().build();
        assert_eq!(repository, RepositoryBuilder::default().build());
        assert_eq!(repository.id.full_name(), "ankh-morpork/watch");
        assert_eq!(repository.description, None);
        assert_eq!(repository.default_branch, "main");
        assert!(!repository.private);
        assert!(!repository.archived);
    }

    #[test]
    fn named_overrides_preserve_the_other_defaults_and_last_value_wins() {
        let id = RepositoryId::parse("lancre/witches").unwrap();
        let repository = RepositoryBuilder::default()
            .id(id.clone())
            .description("Granny Weatherwax's notes")
            .description("Nanny Ogg's notes")
            .default_branch("spells")
            .private(true)
            .archived(true)
            .build();
        assert_eq!(repository.id, id);
        assert_eq!(repository.description.as_deref(), Some("Nanny Ogg's notes"));
        assert_eq!(repository.default_branch, "spells");
        assert!(repository.private);
        assert!(repository.archived);

        let defaults = RepositoryBuilder::default().build();
        assert_eq!(
            RepositoryBuilder::default()
                .description("Watch notes")
                .build(),
            Repository {
                description: Some("Watch notes".into()),
                ..defaults
            }
        );
    }

    #[test]
    fn cloned_builders_and_built_values_are_isolated() {
        let builder = RepositoryBuilder::default().description("Watch notes");
        let unchanged = builder.clone().build();
        let mut changed = builder.clone().description("Patrol notes").build();
        changed.default_branch.push_str("-patrol");
        changed.description.as_mut().unwrap().push_str(" for Vimes");
        assert_eq!(builder.build(), unchanged);
        assert_eq!(unchanged.description.as_deref(), Some("Watch notes"));
        assert_eq!(unchanged.default_branch, "main");
        assert_eq!(RepositoryBuilder::default().build().description, None);
    }
}
