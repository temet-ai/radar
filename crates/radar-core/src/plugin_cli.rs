//! Completion-only CLI facts. No effects, storage handles, host identifiers or
//! clocks cross this boundary. Output and errors are never observation payloads.

use std::future::Future;
use std::pin::Pin;

use crate::journal::{ActionDescriptor, Outcome};

/// The host receives the output and facts together, including on command failure.
pub type ObservedCliTask = Pin<Box<dyn Future<Output = CliExecution> + Send>>;

/// Maximum retained provider observations per CLI execution. Exceeding this bound
/// must not stop or otherwise change the command's actual work.
pub const MAX_CLI_OBSERVATIONS: usize = 256;

/// The ordinary CLI result is independent of its known provider facts. Do not
/// infer the outcome of any observation from this result, or record its contents.
pub struct CliExecution {
    pub result: anyhow::Result<String>,
    pub observations: CliObservations,
}

impl CliExecution {
    pub fn new(result: anyhow::Result<String>, observations: CliObservations) -> Self {
        Self {
            result,
            observations,
        }
    }
}

/// A validated descriptor and explicit terminal fact. Private fields prevent
/// consumers from changing an accepted observation to a nonterminal report.
#[derive(Clone, Debug)]
pub struct CliObservation {
    action: ActionDescriptor,
    outcome: Outcome,
}

impl CliObservation {
    pub fn action(&self) -> &ActionDescriptor {
        &self.action
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub(crate) fn into_parts(self) -> (ActionDescriptor, Outcome) {
        (self.action, self.outcome)
    }
}

/// Bounded observations in provider order. Rejected descriptors, nonterminal
/// outcomes and overflow retain only an omission count, never the omitted data.
#[derive(Clone, Debug, Default)]
pub struct CliObservations {
    observations: Vec<CliObservation>,
    omitted: u64,
}

impl CliObservations {
    /// Retain a safe terminal fact, or count its omission. A false return is
    /// diagnostic only: providers must continue their normal processing.
    pub fn observe(&mut self, action: ActionDescriptor, outcome: Outcome) -> bool {
        if self.observations.len() >= MAX_CLI_OBSERVATIONS
            || !outcome.is_terminal()
            || action.validate().is_err()
        {
            self.omitted = self.omitted.saturating_add(1);
            return false;
        }
        self.observations.push(CliObservation { action, outcome });
        true
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &CliObservation> {
        self.observations.iter()
    }

    pub fn len(&self) -> usize {
        self.observations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    pub fn omitted(&self) -> u64 {
        self.omitted
    }

    pub(crate) fn into_parts(self) -> (Vec<CliObservation>, u64) {
        (self.observations, self.omitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{Origin, SubjectKey, SubjectRole, SubjectSnapshot};

    fn action() -> ActionDescriptor {
        ActionDescriptor::new("probe.read", Origin::Plugin("probe".into()))
    }

    #[test]
    fn cli_observations_preserve_terminal_facts_in_order() {
        let mut observations = CliObservations::default();
        let outcomes = [
            Outcome::Succeeded,
            Outcome::Failed,
            Outcome::Cancelled,
            Outcome::Blocked,
            Outcome::Partial,
            Outcome::Unknown,
        ];
        assert!(observations.is_empty());
        for outcome in outcomes {
            assert!(observations.observe(action(), outcome));
        }
        assert_eq!(observations.len(), outcomes.len());
        assert_eq!(observations.omitted(), 0);
        assert_eq!(
            observations
                .iter()
                .map(CliObservation::outcome)
                .collect::<Vec<_>>(),
            outcomes
        );
        assert!(
            observations
                .iter()
                .all(|fact| fact.action().kind.as_str() == "probe.read")
        );
    }

    #[test]
    fn cli_observations_are_bounded_without_changing_provider_processing() {
        let mut observations = CliObservations::default();
        let mut processed = 0;
        for _ in 0..MAX_CLI_OBSERVATIONS + 3 {
            observations.observe(action(), Outcome::Succeeded);
            processed += 1;
        }
        assert_eq!(processed, MAX_CLI_OBSERVATIONS + 3);
        assert_eq!(observations.len(), MAX_CLI_OBSERVATIONS);
        assert_eq!(observations.omitted(), 3);
    }

    #[test]
    fn cli_observations_reject_nonterminal_and_unsafe_descriptors() {
        let mut observations = CliObservations::default();
        for outcome in [Outcome::Requested, Outcome::Confirmed, Outcome::Started] {
            assert!(!observations.observe(action(), outcome));
        }
        let mut invalid = action();
        invalid.kind = crate::journal::StableCode::new("SECRET unsafe body");
        assert!(!observations.observe(invalid, Outcome::Succeeded));

        let mut invalid = action();
        let mut subject = SubjectSnapshot::new(
            SubjectKey::new("probe", "instance-one", "item", "42").unwrap(),
            SubjectRole::Primary,
        );
        subject.key.id = "x".repeat(8192);
        invalid.subjects.push(subject);
        assert!(!observations.observe(invalid, Outcome::Succeeded));

        let mut invalid = action();
        invalid.metadata.url =
            Some(serde_json::from_str("\"https://host.test/?token=SECRET\"").unwrap());
        // URL metadata is normalized by journal validation. It must not retain
        // a forged, unsanitized URL in the envelope, even if storage is safe.
        assert!(!observations.observe(invalid, Outcome::Succeeded));
        assert!(observations.is_empty());
        assert_eq!(observations.omitted(), 6);
        assert!(!format!("{observations:?}").contains("SECRET"));
    }
}
