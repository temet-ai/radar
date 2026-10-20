---
name: radar-test-consolidation
description: Migrate duplicated or expensive Radar tests to owner-local builders and shared real-host checkpoints while preserving every meaningful assertion. Use for test-suite consolidation, removing incidental rendering or demo setup, and measuring comparable before/after costs. Not a mandate to rewrite already-simple tests.
---

# Consolidate without losing the proof

Read `AGENTS.md` and `docs/test-tiers.md` (its consolidation sections are
the worked examples). Keep unrelated source untouched. The goal
is less setup and stronger, clearer evidence, not fewer tests at any cost.

## Establish the baseline before editing

1. Inventory the selected cases and all their assertions: exact messages/payloads,
   negative checks, ordering, real input/confirmation, effects/tasks, persistence,
   snapshots and dimensions. Map each old guarantee to a retained or new owner.
2. Trace helpers for hidden hosts, demo loads, constructor clock reads, rendering
   on every key, process/DB access and manual task completion. Static counts and
   runtime measurements are different evidence; record which you have.
3. Run the old exact cases with fixed features, toolchain and test concurrency.
   Record commit, command, actual count and several comparable warm observations.
   Separate compilation from execution. If already cheap, justify a migration by
   isolation or correctness rather than promising meaningful speed gains.

## Choose the smallest structural improvement

- Replace a large demo with existing pure owner builders for model/reducer rules.
  Keep configuration and fixture identities coherent; do not add a parallel model.
- Move table rules out of host tests while retaining a real host contract for
  routing, confirmation, capture or renderer behavior that rules cannot prove.
- Share one host only for related sequential guarantees whose earlier state is
  genuinely part of the journey. Keep independent rule cases independent.
- Replace incidental `press` rendering with actual host event/intent dispatch and
  render only preserved checkpoints. Select real emitted targets; do not fabricate
  menu/state success. Retain representative keyboard/mouse routing contracts.
- Prefer `Scenario` over a helper that ignores tracked effects or manually invents
  completion. An ordered `Recorded` response fixture plus exact final ledger is
  useful, but does not become strict pre-response matching by renaming it.

Preserve literal expected requests independently of production request builders.
Retain original golden names, dimensions and output; use scoped snapshot naming
when moving modules. Do not accept regenerated output to make consolidation pass.
Strengthen exact notices or causality checks when they replace weaker coverage,
without hiding the old assertion mapping.

## Prove the migration

Run the new selected cases, then the affected owner and dependent boundary cases.
Inspect the actual diff, including any newly generated snapshot files; discard
only identified tool-generated noise, never unexplained changes. Check named
runtime frame/task limits and exact fixture consumption. Measure the same scope
again; explain any work moved into an already-existing separately executed test.
Do not claim whole-suite speedup from two selected cases.

Classify new narrow scopes and validate real compiled discovery. Confirm old
assertions still have owners, live/ignored and feature-mix coverage remains, and
case-count reductions have an explicit explanation. Ask an independent reviewer
to check the map and hidden fixture effects. Run applicable final gates from the
testing steward; do not rebuild every feature matrix after each tiny active edit.

Deliver a compact mapping, before/after setup and runtime evidence, exact passed
and blocked commands, changed files and any intentionally retained old tests.
Physical unit separation, purity and complete E2E coverage require their own
proof; a new module name or manifest entry does not establish them.
