//! The Copilot adapter passes radar-sessions' shared contract kit.
//!
//! The provider is `sim::Sim`: it acts out each scenario with events copied from the
//! stream recorded live from Copilot CLI 1.0.95 (`fixtures/live-1.0.95.jsonl`).

use std::sync::Arc;

use radar_sessions::contract::{run_all, Harness, Scenario};
use radar_sessions::{Adapter, BoxFuture, RadarSessionId};
use radar_sessions_copilot::adapter::{Clock, CopilotAdapter};
use radar_sessions_copilot::replay::Recording;
use radar_sessions_copilot::sim::{Plot, Sim, SimBackend};

struct CopilotHarness {
    sim: Arc<Sim>,
}

fn clock() -> Clock {
    Arc::new(|| 1_700_000_000)
}

impl CopilotHarness {
    fn new() -> Self {
        let rec = Recording::from_jsonl(include_str!("fixtures/live-1.0.95.jsonl")).unwrap();
        Self { sim: Sim::new(rec) }
    }

    fn adapter_over_sim(&self) -> Box<dyn Adapter> {
        Box::new(CopilotAdapter::new(
            SimBackend(self.sim.clone()),
            None,
            clock(),
        ))
    }
}

impl Harness for CopilotHarness {
    fn adapter(&mut self, scenario: Scenario) -> BoxFuture<'_, Box<dyn Adapter>> {
        // A fresh provider for every scenario; the kit names its sessions `contract-*`.
        let rec = Recording::from_jsonl(include_str!("fixtures/live-1.0.95.jsonl")).unwrap();
        self.sim = Sim::new(rec);
        let plots: &[Plot] = match scenario {
            Scenario::Text => &[Plot::Text],
            Scenario::TwoTurns => &[Plot::Text, Plot::Text],
            Scenario::Tool => &[Plot::Tool],
            Scenario::LongTurn => &[Plot::LongTurn],
            Scenario::Redelivery => &[Plot::Redelivery],
            Scenario::Reordered => &[],
            Scenario::Question => &[Plot::Question],
        };
        for name in [
            "stream", "approve", "approve-session", "deny", "stop", "resume", "resume-waiting", "dup", "order", "lost", "question",
        ] {
            self.sim
                .plan(&RadarSessionId::new(format!("contract-{name}")), plots);
        }
        Box::pin(async move { self.adapter_over_sim() })
    }

    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>> {
        self.sim.restart();
        Box::pin(async move { self.adapter_over_sim() })
    }

    fn settle(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.sim.settled().await })
    }

    fn disconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.sim.disconnect() })
    }

    fn reconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.sim.reconnect() })
    }

    fn release(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.sim.release() })
    }

    fn skips(&self) -> Vec<(Scenario, &'static str)> {
        vec![(
            Scenario::Reordered,
            "Copilot gives events no sequence number: the CLI chains a session's events by parent id and delivers them in order, and radar numbers them as they arrive",
        )]
    }
}

#[tokio::test]
async fn copilot_passes_every_contract_check() {
    let report = run_all(&mut CopilotHarness::new()).await;
    assert_eq!(
        report.skipped.len(),
        1,
        "only out-of-order is skipped: {:?}",
        report.skipped
    );
    assert_eq!(report.skipped[0].0, "out_of_order");
    assert_eq!(report.ran.len(), 12, "{:?}", report.ran);
    assert!(report.ran.contains(&"question"), "{:?}", report.ran);
    assert!(report.ran.contains(&"approve_for_session"), "{:?}", report.ran);
}
