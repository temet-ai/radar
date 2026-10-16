//! The fake provider passes the shared contract kit.

use radar_sessions::contract::run_all;
use radar_sessions::fake::FakeHarness;

#[tokio::test]
async fn the_fake_passes_every_contract_check() {
    let report = run_all(&mut FakeHarness::new()).await;
    assert!(report.skipped.is_empty(), "the fake supports everything: {:?}", report.skipped);
    assert_eq!(report.ran.len(), 13, "{:?}", report.ran);
    assert!(report.ran.contains(&"question"), "{:?}", report.ran);
    assert!(report.ran.contains(&"approve_for_session"), "{:?}", report.ran);
}
