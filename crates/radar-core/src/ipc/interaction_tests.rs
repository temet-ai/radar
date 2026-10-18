//! Contract: verified host motion across optional JSON and coalescing IPC.
//! One owner-local Watch tree; no real providers, tasks or full app frames.

use super::*;
use crate::design::theme::Tier;
use crate::host::Event;
use crate::host::interaction_motion::tests::{ProbeState, fixture};
use crate::view::{Target, TreeRow};
use std::sync::{Arc, Mutex};

fn snap(messages: &[ToRenderer]) -> Option<&Snapshot> {
    messages.iter().find_map(|message| match message {
        ToRenderer::Snapshot(snapshot) => Some(&**snapshot),
        _ => None,
    })
}

fn ready() -> (Bridge, Arc<Mutex<ProbeState>>, u64) {
    let (host, state) = fixture();
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.set_ssh(false);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    let output = bridge.outgoing();
    let cursor = snap(&output).unwrap().cursor;
    (bridge, state, cursor)
}

fn fold(bridge: &mut Bridge, seq: u64, cursor: u64, open: bool) {
    bridge.receive(ToHost::Intent {
        seq,
        cursor,
        intent: Intent::Fold {
            target: Target::new("watch"),
            open,
        },
    });
}

#[test]
fn contract_verified_fold_keeps_origin_and_is_consumed_once() {
    let (mut bridge, state, cursor) = ready();
    fold(&mut bridge, 1, cursor, false);
    assert!(
        snap(&bridge.outgoing()).is_none(),
        "backpressure still holds snapshots"
    );
    bridge.receive(ToHost::Ack { cursor });
    let output = bridge.outgoing();
    let snapshot = snap(&output).unwrap();
    let interaction = snapshot.motion.interaction.as_ref().unwrap();
    assert_eq!(interaction.processed_seq, Some(1));
    assert_eq!(interaction.origin_seq, Some(1));
    let fold = interaction.fold.as_ref().unwrap();
    assert_eq!(
        (fold.origin_seq, fold.origin_cursor),
        (Some(1), Some(cursor))
    );
    assert_eq!(fold.path, vec!["watch".to_string()]);
    assert_eq!(fold.slot, "body");
    assert!(fold.before_open && !fold.after_open);
    assert_eq!(state.lock().unwrap().calls.len(), 1);
    assert!(
        !state
            .lock()
            .unwrap()
            .tree
            .open
            .contains(&vec!["watch".into()])
    );
    bridge.receive(ToHost::Ack {
        cursor: snapshot.cursor,
    });
    assert!(
        snap(&bridge.outgoing()).is_none(),
        "consumption creates no extra snapshot"
    );
    let consumed = bridge.snapshot_now(0).motion.interaction.unwrap();
    assert!(consumed.fold.is_none() && consumed.origin_seq.is_none());
}

#[test]
fn contract_noop_acknowledgement_cannot_be_borrowed_by_later_background_data() {
    let (mut bridge, state, cursor) = ready();
    bridge.receive(ToHost::Ack { cursor });
    fold(&mut bridge, 7, cursor, true); // already open
    let output = bridge.outgoing();
    let snapshot = snap(&output).expect("even a no-op must acknowledge its seq");
    let interaction = snapshot.motion.interaction.as_ref().unwrap();
    assert_eq!(interaction.processed_seq, Some(7));
    assert!(interaction.fold.is_none());
    bridge.receive(ToHost::Ack {
        cursor: snapshot.cursor,
    });
    state.lock().unwrap().tree.rows[0].name = "Watch updated in background".into();
    bridge.update(Event::Tick(1));
    let output = bridge.outgoing();
    let interaction = snap(&output).unwrap().motion.interaction.as_ref().unwrap();
    assert_eq!(interaction.processed_seq, Some(7));
    assert!(interaction.fold.is_none());
}

#[test]
fn contract_noop_while_in_flight_does_not_claim_background_before_ack() {
    let (mut bridge, state, cursor) = ready();
    fold(&mut bridge, 8, cursor, true); // accepted no-op while initial frame is held
    assert!(snap(&bridge.outgoing()).is_none());
    state.lock().unwrap().tree.rows[0].name = "Watch background refresh".into();
    bridge.update(Event::Tick(1));
    bridge.receive(ToHost::Ack { cursor });
    let output = bridge.outgoing();
    let interaction = snap(&output).unwrap().motion.interaction.as_ref().unwrap();
    assert_eq!(
        interaction.processed_seq,
        Some(8),
        "the no-op is still acknowledged"
    );
    assert!(
        interaction.context.is_some(),
        "keep a valid clean identity baseline"
    );
    assert!(
        interaction.origin_seq.is_none() && interaction.fold.is_none(),
        "background state cannot claim the input"
    );
}

#[test]
fn contract_coalescing_keeps_only_final_compatible_fold_and_newer_input_cancels_it() {
    let (mut bridge, state, cursor) = ready();
    fold(&mut bridge, 1, cursor, false);
    fold(&mut bridge, 2, cursor, true);
    fold(&mut bridge, 3, cursor, false);
    bridge.receive(ToHost::Ack { cursor });
    let output = bridge.outgoing();
    let snapshot = snap(&output).unwrap();
    let interaction = snapshot.motion.interaction.as_ref().unwrap();
    assert_eq!(interaction.processed_seq, Some(3));
    assert_eq!(interaction.fold.as_ref().unwrap().origin_seq, Some(3));
    assert_eq!(
        state.lock().unwrap().calls.len(),
        3,
        "all real actions dispatch exactly once"
    );
    fold(&mut bridge, 4, snapshot.cursor, true);
    bridge.receive(ToHost::Intent {
        seq: 5,
        cursor: snapshot.cursor,
        intent: Intent::Select {
            target: TreeRow::target_of(&["watch".into(), "vimes".into()]),
        },
    });
    bridge.receive(ToHost::Ack {
        cursor: snapshot.cursor,
    });
    let output = bridge.outgoing();
    let interaction = snap(&output).unwrap().motion.interaction.as_ref().unwrap();
    assert_eq!(interaction.processed_seq, Some(5));
    assert!(interaction.fold.is_none());
    assert_eq!(state.lock().unwrap().calls.len(), 5);
}

#[test]
fn contract_context_change_with_identical_visible_page_is_sent() {
    let (host, state) = fixture();
    state.lock().unwrap().page = Some("WATCH-12".into());
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.set_ssh(false);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    let first = bridge.outgoing();
    let first = snap(&first).unwrap();
    bridge.receive(ToHost::Ack {
        cursor: first.cursor,
    });
    state.lock().unwrap().page = Some("WATCH-13".into());
    let output = bridge.outgoing();
    let second =
        snap(&output).expect("identity is state even when every displayed word is identical");
    assert_eq!(first.screen, second.screen);
    assert_ne!(
        first.motion.interaction.as_ref().unwrap().context,
        second.motion.interaction.as_ref().unwrap().context
    );
    state.lock().unwrap().keyed = false;
    assert!(
        bridge
            .snapshot_now(0)
            .motion
            .interaction
            .unwrap()
            .context
            .is_none()
    );
}

#[test]
fn contract_interaction_is_full_only_and_reconnect_forgets_acknowledgement() {
    let (mut bridge, _, cursor) = ready();
    fold(&mut bridge, 9, cursor, false);
    for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        bridge.set_tier(tier);
        assert!(bridge.snapshot_now(0).motion.interaction.is_none());
    }
    bridge.set_tier(Tier::TrueColor);
    bridge.set_ssh(true);
    assert!(bridge.snapshot_now(0).motion.interaction.is_none());
    bridge.set_ssh(false);
    for setting in ["subtle", "off"] {
        bridge.host_mut().colours.cfg.motion = setting.into();
        assert!(bridge.snapshot_now(0).motion.interaction.is_none());
    }
    bridge.host_mut().colours.cfg.motion = "full".into();
    bridge.reconnect();
    let interaction = bridge.snapshot_now(0).motion.interaction.unwrap();
    assert!(interaction.processed_seq.is_none() && interaction.fold.is_none());
}

#[test]
fn contract_optional_wire_envelope_defaults_and_round_trips_without_version_change() {
    let legacy: MotionView =
        serde_json::from_value(serde_json::json!({ "level": "full" })).unwrap();
    assert!(legacy.interaction.is_none());
    let (mut bridge, _, cursor) = ready();
    fold(&mut bridge, 1, cursor, false);
    let motion = bridge.snapshot_now(0).motion;
    let json = serde_json::to_value(&motion).unwrap();
    assert_eq!(json["interaction"]["context"]["kind"], "screen");
    assert_eq!(json["interaction"]["fold"]["origin_seq"], 1);
    assert_eq!(
        serde_json::from_value::<MotionView>(json.clone()).unwrap(),
        motion
    );
    // An older consumer's known fields ignore the additive envelope.
    #[derive(serde::Deserialize)]
    struct Legacy {
        level: MotionLevel,
    }
    assert_eq!(
        serde_json::from_value::<Legacy>(json).unwrap().level,
        MotionLevel::Full
    );
    assert_eq!(PROTOCOL, 1);
}
