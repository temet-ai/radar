//! Pure regression checks for named scenario probes; imported once, rather
//! than duplicated in every heavyweight scenario binary.
#[path = "support/probes.rs"]
mod probes;
use probes::{Probe, require, verify};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_failures_keep_checkpoint_and_each_probe_label() {
        let error = verify(
            "reply pending",
            &7,
            &[
                Probe {
                    name: "request",
                    check: |_| Err("wrong body".into()),
                },
                Probe {
                    name: "view",
                    check: |_| Ok(()),
                },
                Probe {
                    name: "journal",
                    check: |_| Err("early terminal".into()),
                },
            ],
        )
        .unwrap_err();
        assert_eq!(
            error,
            "checkpoint reply pending:\nrequest: wrong body\njournal: early terminal"
        );
    }

    #[test]
    fn empty_and_duplicate_probe_sets_cannot_report_success() {
        assert!(require(true, "good").is_ok());
        assert_eq!(require(false, "bad"), Err("bad".into()));
        assert!(verify::<()>("empty", &(), &[]).is_err());
        assert!(
            verify(
                "duplicate",
                &(),
                &[
                    Probe {
                        name: "same",
                        check: |_| Ok(())
                    },
                    Probe {
                        name: "same",
                        check: |_| Ok(())
                    },
                ]
            )
            .is_err()
        );
        assert!(
            verify(
                "named",
                &(),
                &[Probe {
                    name: "one",
                    check: |_| Ok(())
                }]
            )
            .is_ok()
        );
    }
}
