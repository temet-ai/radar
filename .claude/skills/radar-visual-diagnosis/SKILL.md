---
name: radar-visual-diagnosis
description: Diagnose Radar renderer regressions using cheap deterministic fixture, semantic, protocol and text/layout checks before image inspection. Use for Ink checkpoint mismatch, stale or missing visual fixtures, actionable diff artifacts, and deciding what visual evidence is still needed. Not for live-service verification or automatic golden approval.
---

# Spend visual inspection on what machine checks cannot establish

Read `AGENTS.md`, the affected component in `docs/components.md`, and
`docs/scenarios.md`. Keep the exact fixture, dimensions, theme and icon settings
fixed across compared outputs. Ask what failed: state, protocol, content/layout,
style/colour, terminal behavior, or actual image fidelity. These require different
proof; do not send a large screenshot batch to a model to rediscover a text error.

The bounded artifact CLI and exact evidence limits are in
`docs/scenario-renderer-diagnostics.md`. Start with
`scripts/check-scenario-renderers --preflight-only` for input errors; use the
plain command for full producer freshness and Ink replay. The preflight-only
result never qualifies renderer parity.

## Escalate evidence in this order

1. **Inputs and meaning:** reject missing/malformed fixtures and wrong dimensions
   before compiling. Check the owner model/parser and neutral view assertions.
   Cheap structural checks are not complete generated-protocol validation.
2. **Freshness and protocol:** rerun the actual scenario producer against the
   committed fixtures. It must reject stale JSON/text. Never set regeneration
   variables for an ordinary check. For changed protocol types follow the core
   specialist's generation/typecheck path, not a hand-edited TypeScript snapshot.
3. **Actual renderer text/layout:** use `scripts/check-scenario-renderers` for the
   existing real GitHub checkpoint and Ink App parity. Its normalized text evidence
   excludes ANSI styling and trailing whitespace. Read named stage errors and
   expected/actual difference context first; a character position is not necessarily
   a terminal display cell for wide/combining Unicode. Compare code points,
   graphemes and display-cell widths as distinct questions. Do not normalize away
   combining-mark differences unless the production contract defines that
   normalization; preserve raw captures for diagnosis.
4. **Style and images only where needed:** existing `scripts/shots` produces
   deterministic SVG and optional PNG drawn by Ink from sample-data rendering
   (`--compare REF` writes a before and after pair); use it when
   authorized and prerequisites are available. For real terminal Ink behavior, the `radar-ink-live` skill explains
   verified renderer identity, text/colour/mode checks and teardown. Use its offline
   fixture route; this skill does not authorize real-service config or credentials.
   Do not bypass denied socket/process access. Report the unsupported layer.

For a colour/geometry/image question, inspect the actual generated output at the
intended dimensions after deterministic checks. Select only changed relevant
frames/regions with their before/after context. No pixel-equivalence claim from
JSON, normalized text, SVG byte comparison alone or an uninspected screenshot.
Do not invent an image-capture route if existing tools cannot produce one.

## Make the failure useful

Use the diagnostic report's fixture/source identity and stage to reproduce the
smallest failed check. Keep raw expected/actual synthetic text and bounded diff
context in a fresh artifact directory, not new golden files. Link the smallest
artifact that explains the issue; avoid dumping full protocol payloads into
model context. Never capture real user screens, tokens, config or home-directory
state as incidental diagnostics.

A missing tool, producer failure, render timeout or absent actual capture is a
failed/incomplete stage, not a matching empty output. Later stages cannot repair
an earlier freshness failure. Fix the source or fixture-generation cause, rerun
that stage, then dependent stages. Tests must exercise these negative paths.

Golden changes require a deliberate reviewed update: explain intended behavior,
inspect actual diffs, preserve dimensions/identity and rerun both applicable
renderers. Do not add auto-accept flags or silently rewrite baselines. Report
what was checked (semantic/protocol/text/style/image), exact case/command,
first mismatch, artifact paths and all layers still unverified.
