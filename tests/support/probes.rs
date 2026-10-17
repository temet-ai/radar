//! Named, read-only checks over a caller-owned checkpoint snapshot.
//! Setup and transition errors remain fail-fast. Only independent checks that
//! return errors are collected; panics and missing prerequisite state are not
//! caught or hidden. No host, fixture handle, task or global registry is stored.

pub(super) struct Probe<C> {
    pub name: &'static str,
    pub check: fn(&C) -> Result<(), String>,
}

pub(super) fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub(super) fn verify<C>(checkpoint: &str, context: &C, probes: &[Probe<C>]) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    if probes.is_empty()
        || probes
            .iter()
            .any(|p| p.name.is_empty() || !names.insert(p.name))
    {
        return Err(format!(
            "checkpoint {checkpoint}: probes need distinct nonempty names and at least one check"
        ));
    }
    let failures: Vec<_> = probes
        .iter()
        .filter_map(|probe| {
            (probe.check)(context)
                .err()
                .map(|why| format!("{}: {why}", probe.name))
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("checkpoint {checkpoint}:\n{}", failures.join("\n")))
    }
}
