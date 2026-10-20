---
name: radar-scenario-design
description: Design or extend deterministic Radar regression scenarios, owner-local starting-state fixtures and named checkpoint probes. Use when adding a real-host journey, shortening repeated setup, or extending related verification in an existing expensive test. Not for live provider recording or arbitrary UI automation.
---

# Design a scenario that proves the intended boundary

Read `AGENTS.md`, `docs/test-tiers.md` and the relevant recipe in
`docs/scenarios.md`. Inspect the actual owner fixture and
`crates/radar-core/src/scenario.rs` before adding a helper. Prefer the cheapest
boundary that can expose the regression; a parser or pure reducer assertion does
not need a host. Root tests compose parts; owner fixtures never import siblings.

## Define the claim before the setup

Write a short contract: starting state, actual transition, external response,
observable checkpoint and forbidden effects. Identify which assertions require
real host routing, task completion, SQLite/Git or renderer output. Keep those
collaborators real; supply only external responses through existing owner seams.

Choose one honest starting-state method:

- Direct initialization with owner-local builders and fixed values, such as Jira
  `TicketSelected`. This proves a state, not that refresh/auth/persistence ran.
- Real transition replay, such as `tests/support/slack_checkpoints.rs`. Return a
  named checkpoint after production dispatch and completion have actually run.

Keep a representative independent real-flow equivalence test for direct shortcuts.
Expose explicit per-test overrides instead of a growing universal demo world.
Constructors that read the clock or environment are not pure even if fields are
later overwritten. Do not assign private success, authentication or journal
terminal fields to simulate a transition the test claims to execute.

## Execute once, verify related facts cheaply

Use `Scenario::dispatch` and bounded `settle`, `settle_until` or `settle_io`;
advance fixed time explicitly. Use `inspect` for mode-scoped host inspection.
Assert held/pending state before release, then parsed/reduced final state and
idle completion. Propagate unsupported-effect errors and finish journals.

Build an owned read-only checkpoint snapshot. Add related named checks using
`tests/support/probes.rs` (`Probe`, `verify`): request identity/count, view/draft,
action causality, privacy and persistence. Names must identify failing guarantees.
Aggregate independent returned failures only; prerequisites and panics fail fast.
Do not create cross-test state, hide dispatch in a probe or turn unrelated
journeys into one order-dependent omnibus test.

Use exact owner transport expectations, including origin, params/body and write
intent. Legitimate concurrent reads need not have a fixed request order. Cover
relevant refusal/partial/stale/cancel/duplicate/lost-response paths and no retry
before adding another happy-path host. Distinguish local draft cancellation, a
cancellation request, a task actually aborted, and provider acceptance with a late
or lost response. Test the transition the available API actually supports; do
not invent remote cancellation or deliver-after-abort behavior. Treat provider
acceptance separately from client knowledge. For Slack, inspect
`crates/radar-slack/src/reply_context_tests.rs`, especially
`pending_reply_detail_cannot_edit_drop_or_confirm_the_inflight_operation`,
before assuming an in-flight send can be cancelled: a pre-send draft drop and
a late completion after navigation are different regressions. Distinguish fresh-Host cache restore from process restart and
database durability. Keep secrets and authored payloads out of journal evidence
where the production privacy contract excludes them.

## Finish with executable evidence

- Run the exact case with its minimal explicit features. `scripts/scenarios --list`
  shows supported recipes; `--dry-run` prints a recipe's fixed command without
  execution. The menu is replay selection, not a graphical debugger.
- Update affected narrow scopes in `scripts/test-tiers.json`; validate compiled
  discovery using `docs/test-tiers.md`. Do not treat policy validation as execution.
- Assert `Scenario::costs()` at the intended checkpoints. Its shell frames are
  host-owned; neutral views and Ink are outside that counter.
- Verify fixture completion/ledger and teardown, then affected owner/root tests.
  Use the steward's acceptance checklist for final gates and resource safety.

Deliver the contract, why a cheaper test cannot replace it, direct-versus-replayed
setup distinction, exact executed case/features, negative evidence, frame/cost
scope and remaining limits. Never imply live-service or pixel fidelity from a
strict fake and neutral/text view.
