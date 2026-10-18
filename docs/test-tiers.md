# Explicit test tiers and discovery gate

See [GUIDE §3](GUIDE.md#3-testing) for the existing pyramid and budgets, and
[headless scenarios](scenarios.md) for the replayable journeys.

## What this gate establishes

`scripts/test-tiers.json` enumerates individual compiled test names in **strict
migrated scopes**. `scripts/check-test-tiers` compares those declarations with
real test-harness discovery. It rejects unclassified additions within those
scopes, missing declarations, missing binaries/features, overlapping scopes,
unknown tiers, and heavy cases without a protected boundary, minimal parts and
reason unit coverage is insufficient. With nextest discovery it also rejects
changes to a case's ignored state. No filename is treated as a unit guarantee.

Everything discovered outside the selected scopes remains visible as an
**unclassified backlog**. `--report FILE` writes every classified and unclassified
case. `--reject-backlog` rejects that backlog too; it is deliberately not the
default while legacy tests are still mixed. This incremental gate does not yet
reject new tests outside the migrated scopes. Review new areas explicitly and
add narrow scopes as they are classified. Do not add a wildcard “unit” bucket or
rename a case out of a protected scope merely to pass the gate.

This is a local inventory gate, not an installed CI job. Run it when changing a
classified area. It does not run or remove tests, alter feature-mix coverage, or
turn on ignored live tests. A source grep cannot prove what a helper calls: this
checker uses compiled discovery for names but **does not prove purity**. The
architecture guard in `tests/parts.rs` separately checks Cargo dependency edges,
including target-specific and dev/build dependencies and named adapter-owner
exceptions. Owner-local pure fixtures still require code review and eventual
physical production-boundary extraction. `radar-core` currently combines host,
rendering, storage and contracts, so linking core is not unit-lane isolation.

## Tier ownership

The `core-focus-ramp-kit` scope classifies the color sampler, clockwise perimeter,
protected-title geometry and wordmark helpers. These use explicit themes and
caller-owned local buffers, without a Host or full application frame. Its three
Block/card contracts intentionally compose production painters to prove clipped
titles and legacy inner geometry survive, and scrollbars paint last. The
`core-focus-ramp-view` contracts paint real screens to establish caret ownership,
page/work-area dock eligibility and exact lower-tier preservation. The
`core-focus-ramp-shell` unit cases cover pure chrome eligibility and wordmark
span/clipping behavior. Cross-renderer App parity requires separate execution
evidence.

- **Unit:** one component's pure model, parsing or effect-envelope behavior. No
  real host, full frame, wall-clock reads, network, credential lookup, filesystem/database, process
  or mutable global fixture state. Async does not alone make a test integration:
  the completion mapper's tests use local futures and synthetic events.
- **Integration:** a small number of real collaborating components, such as a
  host and one plugin. Disposable real SQLite/Git is appropriate when persistence
  or repository semantics are the assertion. Only root tests compose sibling
  parts; fixture builders stay in their owner.
- **Contract:** a named public/protocol/renderer/transport boundary that lower
  tiers cannot establish. Existing whole-app parity sweeps remain covered and
  explicitly documented; avoid growing them for assertions available at unit
  speed. A transport contract need not be slow.
- **E2E:** a minimal real process/feature-mix journey. Feature compilation coverage
  is separate from scenario coverage; retain full `scripts/check-mixes` at merge.
- **Live:** explicitly ignored real-service checks, outside offline execution.
  Do not use `--run-ignored all` or `--include-ignored` for inventory validation.

Names-only tab coverage has four narrow core scopes. `core-tab-attention-host`
is integration: the host combines readiness, grouping, plugin badges and unsaved
settings into chrome attention while preserving status data. The
`core-tab-attention-shell` contract inspects the real Ratatui top bar's cells,
mnemonics and click targets; `core-tab-attention-wire` protects JSON attention
fields and their absent-field defaults without constructing a host.
`core-page-tab-rows` classifies the complete small `ui::page::tests` module as
unit: its cases use caller-owned tab values, local row-sized buffers and local
hit collections. They do not construct a host, render a full application frame,
read the clock, access persistence/network, or spawn a process. A local buffer
alone does not make an isolated layout/hit-helper case integration. Ink rendering
and cross-renderer parity remain separate checks; these declarations do not
establish that either renderer's suite has run.

The owner-local `gitlab-page-tab-attention` unit scope uses `MrBuilder` and
owned detail/discussion values to distinguish unresolved work and failed
pipelines from ordinary totals. It constructs no plugin, host, demo or frame.

The two Jira builder migration comparisons remain **contract** tests: their
legacy side constructs a full demo plugin, whose constructor reads the clock
before demo time is overwritten. They protect the narrowed model inputs against
legacy setup; they are not pure-builder-only tests. GitLab's corresponding filter
comparison uses owner-local model values and remains unit. Core's scratch Work
and History tests use production writers/readers and are integration, while the
absent-journal completion mapping is unit (no host or database). The GitHub root
journey is a contract case because it additionally renders one real Ratatui frame
and checks scenario-derived JSON/text; external Ink verification is separate.

The Jira filter migration separates three owner-local reducer cases under
`tests::filter::unit` from one captured-typing host contract. The reviewed unit
setup uses `Default`, fixed time and owned `TicketBuilder` values; it does not
construct the legacy demo plugin. The contract retains one host and one full
Ratatui frame. These source-level construction counts are not runtime measurements
or compiler-enforced purity. The root `local-work-journey` is integration: it
requires `git,workbench`, disposable real Git subprocesses and SQLite with two
plugins, without full-frame rendering. Its declaration is an inventory entry,
not evidence that execution has passed. All nine owner-local Git fixture cases
are integration, including negative checks, because their setup creates real
scratch repositories and runs trusted installed Git. Explicit child environments,
protocol restrictions and owned-parent checks are not an operating-system sandbox.

GitLab confirmation migration similarly separates three fixed-time owner-local
question/effect cases from one host contract. The latter preserves rebase/merge
menu routing, held provider responses and production refresh ingestion with one
full-frame pending-wording checkpoint and no SQLite. Source consolidation counts
are distinct from execution timings and the measured per-host frame counter.

The selected Jira checkpoint has four owner-local unit cases for selected-state
values, neutral views/intents and identity replacement, one injected-client
lifecycle contract, and one real-host refresh equivalence integration case. The
reviewed direct path uses fixed-time owned state and does not construct a host,
render a frame or execute provider calls. Direct checkpoint initialization must
never invent successful refresh history; the executed equivalence path supplies
that separate evidence. Classification reflects these reviewed call paths, not
an automated purity proof.

The Sessions fake-inspection scope has four adapter contracts using in-memory
Tokio sequencing. Its root permission journey is integration because it combines
the host, a dedicated runtime and SQLite. Fake shell events establish permission
handling, not actual shell execution or filesystem changes; production setup
still reads PATH metadata and the current directory. Slack's complete root
binary scope contains four hosted integration journeys. The two pure captured-
snapshot probe-helper unit tests live once in the separate `scenario_probes`
binary, avoiding duplication as multiple journeys reuse the helpers. Snapshot
probes cannot drive a host or release responses, and empty probe sets must fail.

The [Slack reply consolidation](plans/slack-reply-consolidation.md) adds two
narrow contract scopes. The shared host case retains one demo-backed host and
exactly two real frames protecting the unchanged typing/confirmation goldens,
while requiring no provider requests. The owner acceptance case uses fixed owned
state and the real reply tasks/reducers against `Recorded` transport, with no host,
demo or frame. It checks exact preview transmission and subsequent thread read;
it does not claim bootstrap, authentication or persistence. The preserved root
journeys still protect provider/journal lifecycle and exact 429 uncertainty.
Consolidation source counts and passing inventory are not execution timings.

The [Slack review-draft consolidation](plans/slack-review-consolidation.md)
adds one narrow host contract. One real host retains six unchanged goldens and
the prior escaped-text frame, with an actual seven-frame lifetime assertion.
Local Code/Tracker contract stubs are not real sibling-part dependencies. The
Recorded ledger stays empty and dispatch rejects provider work; edit/drop routing
and rendered content are the boundary, not a successful external review. Source
inventory and frame limits are separate from measured runtime verification.

The [Slack review-post consolidation](plans/slack-review-post-consolidation.md)
protects success and refusal with one sample-backed host, two actual production
post tasks/reducers under `Scenario`, and exactly two lifetime shell frames. It
retains the followed Code-row render and unchanged refusal golden. `Recorded`
supplies ordered responses; complete literal ledgers and queue exhaustion are
checked afterward. This is not strict pre-response request matching, actual
bootstrap, persistent journal verification or sibling-part crate composition.
Execution and before/after timings belong to the separate verification lane.

The Slack accepted-post/response-loss fixture has three client/transport
contracts and one pure constructor-validation unit case. The hosted integration
keeps synthetic server acceptance distinct from client knowledge and expects the
first journal outcome to remain Unknown. A fresh in-process host restores the
real serialized cache snapshot, reads the accepted message and must not resend;
it has a separate scratch journal. This scope does not claim process restart,
cache-database or same-journal recovery, or durable draft restoration. These are
the assertions under test, not a statement that reconstructed code has passed.

The root `review-refusal-journey` requires only `gitlab,sessions` and real SQLite.
It drives the selected merge request through production detail parsing and the
host's missing-worktree-provider refusal, with captured-state probes. No Git part,
worktree creation, actual agent tool execution or successful review is simulated.
The expected product gap is the assertion, not a successful-review substitute.
Production setup still performs the documented read-only environment lookups.

The complete `parts-architecture` scope explicitly classifies 17 cases: ten
synthetic metadata/string/feature-closure checks are unit, while seven actual
metadata/source/Cargo-tree checks are architecture contracts. No runtime plugin
is constructed. Source-name checks remain supplementary vocabulary checks, not
dependency-isolation proof. The default/demo declaration analysis expands root
features from actual no-deps metadata; it is not a resolved full default graph.
Separate actual Cargo-tree contracts cover fourteen no-owner/individual-owner
selections with support off/on and the `demo`/`slack,demo` selections. A test
declaration or passing inventory does not establish execution of those checks.
Full-default resolved-graph verification remains separate.

Every non-unit case records the protected boundary, minimal involved parts and
why a unit assertion cannot cover it. These are reviewed declarations; manifest
validation does not enforce cost. The test-support-only `Scenario::costs()` now
reports driver-dispatched events, returned effect envelopes, spawned completion
tasks and lifetime `shell::render` calls on its owned host. Frame assertions in
selected root journeys enforce their own zero/one-frame boundary at runtime,
including calls through raw shell helpers. Neutral views, part-only buffers, Ink
and other hosts are excluded. Host construction counts remain uninstrumented;
event/task counts do not cover detached plugin work or prove complete idleness.
Two explicit core regressions protect these wiring points: driver/task accounting
is integration, and exercising every raw shell helper is a renderer contract.
The latter intentionally renders several frames to prove coverage; it is not a
zero/one-frame product journey. These counters measure work, not elapsed time,
purity or sandbox isolation. Existing
whole-app cases list the complete set of participating part kinds rather than
claiming a one-part fixture. No shared all-parts fixture is introduced.

## Run against nextest discovery

Python 3 with its standard library is sufficient for the checker. Nextest is
preferred because its test list includes ignored state. Discovery may compile;
coordinate it with the single Cargo build lane and use the repo's `target/`.
Do not filter individual tests in discovery or use `--list-type binaries-only`.
Feature/package selection is allowed when the corresponding scopes are explicit.

Root hosted scenario binaries require explicit root `test-support` alongside
their actual part features. Weak owner forwarding must enable fixtures only for
parts already selected, never select all providers implicitly. The provider
scenario target also rejects test-support with no selected provider instead of
producing an empty successful binary. `scenario_probes`
remains part-free and needs no test-support feature. Default or no-default ad hoc
test commands may omit fixture binaries when their required features are absent;
they are not complete scenario discovery. The demo/shots path can retain core
support for legacy helpers but must not expose owner fixtures through root test
support. Compatibility test invocations enable the feature explicitly without
changing shipped feature checks.

The current complete `core-scenarios` and `git-checkout-fixtures` inventories
target Unix (Linux/macOS): the file/symlink preservation and Git symlink/hard-link
regressions are compiled under `cfg(unix)`. A non-Unix listing will fail with
those declared cases missing. Do not silently waive it or
claim the full scope passed; platform-aware manifests are not implemented yet.

```sh
# Cheap checker/config regressions; no Rust build and no coverage assertion:
python3 scripts/test-tier-tests.py
scripts/check-test-tiers --validate-policy

# One owner, all tests discovered, only its migrated module required:
cargo nextest list --locked -p radar-github --lib --message-format json > /tmp/github-tests.json
scripts/check-test-tiers --nextest /tmp/github-tests.json --scope github-models --report /tmp/github-tiers.json

# Owner-local client fixtures (Jira's complete scope includes its demo regression):
cargo nextest list --locked -p radar-github --lib --features test-support,demo --message-format json > /tmp/github-tests.json
scripts/check-test-tiers --nextest /tmp/github-tests.json --scope github-models --scope github-hosted-fixtures
cargo nextest list --locked -p radar-gitlab --lib --features test-support,demo --message-format json > /tmp/gitlab-tests.json
scripts/check-test-tiers --nextest /tmp/gitlab-tests.json --scope gitlab-fixtures --scope gitlab-filter-builder-parity --scope gitlab-confirmation-unit --scope gitlab-confirmation-contract --scope gitlab-page-tab-attention
cargo nextest list --locked -p radar-jira --lib --features test-support,demo --message-format json > /tmp/jira-tests.json
scripts/check-test-tiers --nextest /tmp/jira-tests.json --scope jira-fixtures --scope jira-start-builder-parity --scope jira-dispatch-builder-parity --scope jira-filter-unit --scope jira-filter-host-contract --scope jira-selected-checkpoint

# Root journeys have one strict module scope per enabled provider:
cargo nextest list --locked -p radar --no-default-features --features github,gitlab,jira,test-support --test provider_scenarios --message-format json > /tmp/provider-tests.json
scripts/check-test-tiers --nextest /tmp/provider-tests.json --scope github-journeys --scope gitlab-journeys --scope jira-journeys

# Core migrated areas need their explicit test-support feature:
cargo nextest list --locked -p radar-core --lib --features test-support --message-format json > /tmp/core-tests.json
scripts/check-test-tiers --nextest /tmp/core-tests.json --scope core-completion --scope core-scenarios

# Names-only tabs: the same unfiltered core discovery checks all four narrow scopes:
scripts/check-test-tiers --nextest /tmp/core-tests.json \
  --scope core-tab-attention-host --scope core-tab-attention-shell \
  --scope core-tab-attention-wire --scope core-page-tab-rows

# Owner-local disposable checkout fixture discovery (real subprocess tests):
cargo nextest list --locked -p radar-git --lib --features test-support,demo --message-format json > /tmp/git-tests.json
scripts/check-test-tiers --nextest /tmp/git-tests.json --scope git-checkout-fixtures

# Local Git/Work journey discovery uses its two required part features:
cargo nextest list --locked -p radar --no-default-features --features git,workbench,test-support --test local_scenarios --message-format json > /tmp/local-tests.json
scripts/check-test-tiers --nextest /tmp/local-tests.json --scope local-work-journey

# Existing fake-provider adapter contracts and its hosted permission journey:
cargo nextest list --locked -p radar-sessions --lib --message-format json > /tmp/sessions-tests.json
scripts/check-test-tiers --nextest /tmp/sessions-tests.json --scope sessions-fake-inspection
cargo nextest list --locked -p radar --no-default-features --features sessions,test-support --test agent_scenarios --message-format json > /tmp/agent-tests.json
scripts/check-test-tiers --nextest /tmp/agent-tests.json --scope agent-permission-journey

# Slack owner reply contracts, including two unchanged golden checkpoints:
cargo nextest list --locked -p radar-slack --lib --features test-support,demo --message-format json > /tmp/slack-owner-tests.json
scripts/check-test-tiers --nextest /tmp/slack-owner-tests.json --scope slack-cli-fixtures --scope slack-hosted-fixtures --scope slack-reply-host-contract --scope slack-reply-acceptance-contract --scope slack-review-draft-contract --scope slack-review-post-contract

# Slack hosted journeys and separate pure reusable probe helpers:
cargo nextest list --locked -p radar --no-default-features --features slack,test-support --test slack_scenarios --message-format json > /tmp/slack-tests.json
scripts/check-test-tiers --nextest /tmp/slack-tests.json --scope slack-journeys
cargo nextest list --locked -p radar --no-default-features --test scenario_probes --message-format json > /tmp/probe-tests.json
scripts/check-test-tiers --nextest /tmp/probe-tests.json --scope scenario-probes

# GitLab/Sessions review stops at the real missing-worktree-provider refusal:
cargo nextest list --locked -p radar --no-default-features --features gitlab,sessions,test-support --test review_scenarios --message-format json > /tmp/review-tests.json
scripts/check-test-tiers --nextest /tmp/review-tests.json --scope review-refusal-journey

# Complete architecture binary: synthetic units and actual graph contracts:
cargo nextest list --locked -p radar --no-default-features --test parts --message-format json > /tmp/parts-tests.json
scripts/check-test-tiers --nextest /tmp/parts-tests.json --scope parts-architecture

# Full discovery requires features that include all declared root targets:
cargo nextest list --locked --workspace --all-features --message-format json > /tmp/radar-tests.json
scripts/check-test-tiers --nextest /tmp/radar-tests.json --report /tmp/radar-tiers.json
```

Package/library/binary names come from nextest's
[`TestListSummary`](https://nexte.st/docs/machine-readable/list/). Scope selection
is intentionally explicit: an absent feature-gated test is an error, not a
silently successful empty match. Omitting `--scope` requires **all** manifest
scopes, even if the supplied discovery only includes one crate. Discovery checks
only the supplied build/feature configuration; another configuration may expose
other tests. Keep feature-mix checks independently.

Without nextest, save the output of **one freshly built test executable** with
`--list --format terse`. Obtain the executable path from Cargo's build artifact
output (`cargo test --no-run --message-format json`), not an arbitrary matching
old executable in `target/`. Do not pass combined `cargo test -- --list` output:
that loses binary identity and contains prose headers.

```sh
"$TEST_EXECUTABLE" --list --format terse > /tmp/github-tests.list
scripts/check-test-tiers --libtest /tmp/github-tests.list \
  --package radar-github --binary radar_github --scope github-models
```

Libtest text does not expose ignored state. The checker reports that limitation
rather than treating missing metadata as “not ignored.” It rejects empty input,
unexpected lines, duplicate tests and benchmark listings. It never invokes the
binary itself. Test execution remains a separate command, e.g.
`cargo nextest run --locked -p radar-github`.

## Runtime limits and migration discipline

The existing nextest configuration is unchanged: slow at 2 seconds and terminate
at 20 seconds; only the already named `key_intent_parity` and `ratatui_cells`
binaries use 10/60 seconds. Keep roughly 10 seconds per crate and 3 minutes for
warm mix checks. Cargo test alone does not enforce nextest hard timeouts. This
inventory gate measures no runtime and supplies no new timeout exemptions.

Before calling an area “fast” or isolated, measure repeated warmed execution with
unchanged feature configuration and coverage, separating build time from test
execution. Before accepting heavy cases, review the actual host/frame count and
whether a smaller part-local assertion suffices. Neither timing guesses nor a
passing name inventory is evidence of purity or a speed improvement.

To migrate another area: inspect each test and its helper calls; define a narrow
module-prefix or complete-binary scope; enumerate every discovered case and its
tier; add its boundary rationale; run discovery plus the checker; and execute the
appropriate owner-local tests. Missing cases and new unclassified cases must be
resolved explicitly, never by deleting coverage or raising all budgets.


## Command palette coverage

The `core-palette-matching`, `core-palette-host`, `core-palette-shell` and
`core-palette-wire` scopes separate pure matching, host routing, Unicode layout,
and protocol serialization. Each of GitLab, GitHub, Jira, Slack and Git has an
owner-local `*-palette-targets` unit scope for cached inventory and scoped routing.
The host's raw-mouse case is a renderer contract, not a unit: it constructs the
real shell hit map and proves a second press cannot confirm a replacement menu.

For focused visual diagnosis, that same contract accepts
`RADAR_PALETTE_ARTIFACTS=/absolute/output/directory` to export synthetic discovery
and search checkpoints at 80×24 and 160×40: real Ratatui SVG/text and Bridge JSON.
Ordinary checks write nothing. Replay the JSON through the existing Ink App test
harness to inspect the actual frontend; these artifacts do not establish live
terminal/PTY behavior or pixel equivalence.
