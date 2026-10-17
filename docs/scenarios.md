# Headless scenario replay

`scripts/scenarios` is a small developer launcher for ten hosted
journeys in `tests/slack_scenarios.rs`, `tests/provider_scenarios.rs`,
`tests/local_scenarios.rs`, `tests/agent_scenarios.rs` and `tests/review_scenarios.rs`.
It selects a scripted test and replays it through the real host, reducer, task
completion path and disposable SQLite journal. Python 3.9+ and the repository's
working Rust/Cargo toolchain are required; no Python packages are needed.

```sh
scripts/scenarios --list
scripts/scenarios slack-success
scripts/scenarios slack-refusal
scripts/scenarios slack-uncertain
scripts/scenarios github-denied-detail
scripts/scenarios gitlab-refresh
scripts/scenarios jira-partial-refresh
scripts/scenarios git-work-link
scripts/scenarios agent-permissions
scripts/scenarios review-no-worktree
scripts/scenarios slack-success --dry-run
scripts/scenarios --interactive
python3 scripts/scenarios-tests.py  # launcher checks only; never builds Rust
```

In the text menu, enter a listed number or name. `r` or Enter replays the previous
selection, and `q` or EOF exits. Invalid input launches nothing. Every replay
starts a new test process, host, strict provider fixture and scratch journal;
there is no persistent scenario session to reset. The menu exits nonzero if any
replay failed, even if a later replay succeeds. Ctrl-C exits with status 130.

## Named recipes

| Name | Scripted outcome and assertions |
| --- | --- |
| `slack-success` | Refresh and select a real feed row, compose and confirm a reply, observe the held provider response, release success, refresh the thread and verify a persisted `Succeeded` outcome. |
| `slack-refusal` | The same refresh/selection/composition flow in read-only mode; confirmation refuses every write, retains a recoverable draft and persists `Blocked`. |
| `slack-uncertain` | Release a held synthetic HTTP 429 after confirmation; retain the draft, report the uncertain outcome, prove no silent write retry and persist `Unknown`. |
| `github-denied-detail` | Release a held pull-request refresh, activate a row, release HTTP 403 detail denial, return to the preserved list, prove no silent retry and persist read success/failure plus item count. |
| `gitlab-refresh` | Release a held full refresh, verify the merge request in the real view and persist successful refresh, merge request and project counts. |
| `jira-partial-refresh` | Release a held response with one valid ticket and one missing its key, show only the valid ticket and persist `Partial` with retained/omitted counts. |
| `git-work-link` | Read a disposable real checkout, review and confirm its Work link, persist exact source/history evidence and reopen the unchanged checkout. |
| `agent-permissions` | Drive a fake-agent turn through actual dock denial, then approve a fresh permission; retain both decisions and persist actual dispatch evidence. |
| `review-no-worktree` | Hold a real GitLab detail read, accept its review request, then verify the actual missing-worktree refusal without creating an agent or review artifact. |

Fault selection means choosing one of these fixed recipes. The launcher does not
inject arbitrary errors, pause at runtime breakpoints, step through a live UI,
launch Ink or Ratatui, or export Storybook checkpoints. Its interactive option is
only a text selection/replay loop. Passing these tests does not verify renderer
layout or a real provider's behavior.

## Execution and diagnostics

The command is printed before execution. A replay uses the equivalent of:

```sh
cargo test --locked --offline --color never --target-dir /path/to/gg/target \
  -p radar --no-default-features --features slack,test-support --test slack_scenarios \
  -- --exact confirmed_reply_waits_for_provider_then_refreshes_and_persists_success \
  --nocapture --test-threads=1
```

Cargo incrementally reuses the repository's `target/`; nothing is cleaned or
installed. Default features are disabled. Every recipe enables root `test-support`
and only its actual part features. Fixture support weakly forwards to selected
owners, so selecting it alone adds no optional part. The local Git/Work recipe
selects `git,workbench,test-support` and `local_scenarios`; the fake-agent recipe
selects `sessions,test-support` and `agent_scenarios`; the review refusal selects
`gitlab,sessions,test-support` and `review_scenarios`.
The four Slack recipes use `slack_scenarios`; GitHub, GitLab and Jira use
`provider_scenarios` with the exact module-qualified test name. Binary and feature
selection are fixed in the launcher allowlist, with no CLI passthrough. Root
scenario targets declare Cargo `required-features`, so invoking a heavy binary
without `test-support` reports the missing feature. `provider_scenarios` supports
any one of its three provider features, selected by its module gates. The pure
`scenario_probes` target needs neither parts nor `test-support`. Existing
mix-test invocations select `demo,test-support`; the production check/build
feature lists and exhaustive part enumeration are unchanged. `demo` and default
`shots` do not enable owner fixture support in shipped builds. `--all-features`
still includes every scenario. Ad hoc whole-root test runs, with or without
default features, omit the gated fixture binaries unless `test-support` is
selected. Run them via this launcher, `--all-features`, or `scripts/check-mixes`;
`mix`, `reference_views` and `github_links` explicitly require fixture support,
and `slack_cli_history` requires `slack,test-support`. `--locked`
protects the dependency graph, and `--offline` prevents dependency fetches. If
required dependencies are missing locally, prepare the usual development build
separately; the launcher reports the Cargo failure and does not install a toolkit
or retry online. First compilation can take much longer than a warm replay.

Use `--cargo /path/to/cargo` for a specific executable. This accepts one executable,
not a shell command or arbitrary Cargo arguments. Caller environment settings
such as `PATH`, `RUSTUP_TOOLCHAIN`, `RUSTFLAGS`, `CARGO_BUILD_JOBS` and linker
configuration are inherited unchanged. The explicit `--target-dir` deliberately
keeps this lane in the repository target even if `CARGO_TARGET_DIR` is set.
Repository Cargo configuration still applies; on Linux it currently selects mold
unless the caller's existing configuration overrides it.

While Cargo runs, output is captured in a temporary file. After completion the
launcher prints at most its final 16 KiB, then removes the log. This bounds
in-memory/displayed diagnostics, not log disk usage. On failure it preserves
Cargo's positive exit status. Missing Cargo produces an actionable error. A zero
Cargo exit is accepted only if the harness reports exactly one passing,
non-ignored test, so a stale name matching no tests cannot look successful.

The launcher does not set or loosen test timeouts. It uses `cargo test`, which
does not apply nextest's runtime budgets, and imposes no command wall timeout
that would conflate compilation with execution. The scenarios retain their own
bounded scheduling/settle steps. Those steps are not an OS sandbox and cannot
bound arbitrary synchronous reducer code.

## Isolation and coverage

The hosted provider fixtures inject strict scripted transports before credential
resolution, check exact requests and unused expectations, and fail on unsupported
provider activity. They use synthetic fixture configuration rather than live
accounts. The launcher does not start the real radar application, access provider
credentials, change global environment/security settings, or connect live services.
The safety boundary comes from the test fixture and narrow feature selection,
not merely from `RADAR_SANDBOX` or Cargo's offline flag.

Each test uses a fresh temporary SQLite database and checks semantic persisted
outcomes. Normal teardown removes it; a hard-killed process can leave its scratch
directory in the system temp directory. SQLite session IDs/timestamps are real
persistence values and are not promised byte-identical on replay. Config/cache
writes are captured in memory; desktop effects are captured and these particular
recipes assert that none were requested. No assertion claims a real desktop
operation completed.

For the driver contracts, strict fixture rules, current limitations and broader
verification lanes, see [the fixture plan](plans/testing-fixtures.md). These ten
protected integrations supplement the unit suite, feature-mix checks and future
renderer checkpoints; they do not replace them.

## One real renderer checkpoint

`github-denied-detail` also verifies the committed 120×40 GitHub refresh checkpoint
against a new run. The JSON contains the actual host chrome, screen and focus; the
text is the actual Ratatui frame at the same point with icons off. The focused Ink
App test draws that JSON and compares visible text rows with Ratatui. It does not
check cell colors or modifiers for this checkpoint.

After installing the locked dependencies in `ink/`, run:

```sh
scripts/check-scenario-renderers
```

The command runs the producer first and refuses to regenerate expectations during
a check. Regeneration is explicit through `RADAR_SCENARIO_FIXTURES_DIR`, which must
name a new directory; inspect the JSON and text before accepting changes. This
is one headless renderer contract, not a socket/PTY end-to-end session or a
Storybook preview.

## Disposable Git and Work

`git-work-link` requires an installed Git executable. Its owner-local fixture
creates a fresh seed repository, bare local origin and working clone under one
caller-owned temporary directory, then reads actual dirty checkout state. Git
children receive isolated configuration, hooks and environment; unsupported
commands and paths fail closed. No provider account or remote network is used.
The Work confirmation executes the production SQLite mutation path. The test
checks source bindings and associated read evidence, and verifies that the
checkout remains unchanged after fixture setup. A user-asserted Work link does
not establish a code change or task completion.

## Start at a named checkpoint

Root-owned helpers in `tests/support/slack_checkpoints.rs` let focused cases
start with `at_open_thread(...)` or `at_reply_confirmation(...)`. The latter
executes the same real refresh, row selection and composition path; its name
shortens the test, not the execution semantics. Refusal and uncertain-reply cases
reuse it, supplying only write mode and the required response override. The
protected success journey retains the explicit open-thread → compose → confirm
steps. Every call creates fresh state; there is no shared host or serialized
journal replay. The caller retains the scratch directory for the whole run.

A direct-initialized owner fixture is a different kind of starting point: it
must say which state is supplied, omit history for transitions that never ran,
and have an equivalence contract against the real path for the behavior being
asserted. Do not use a convenient prefilled state as evidence that a refresh,
confirmation, mutation or restart succeeded.

## Get more checks from the same journey

Add related checks at an existing protected checkpoint instead of starting a
second host for each assertion. The Slack held-reply checkpoint captures owned
request records, journal records, the neutral view and notice once. Named probes
then check the exact held request, nonterminal action, retained thread, absent
premature success and desktop isolation. Both success and uncertainty reuse this
probe set. After the real SQLite writer closes, a separate persisted checkpoint
checks the action identity, request/confirmation, exact terminal outcome and
exclusion of message text.

For a new guarantee:

1. Choose the existing journey and checkpoint that already reaches the relevant
   state. Keep unrelated behaviors in their own focused tests.
2. Add the smallest owned observation needed to its snapshot. Probes receive no
   host, fixture handle or mutable state; they cannot dispatch or release replies.
3. Add a uniquely named `Probe { name, check }` in that checkpoint's list. Return
   a useful error instead of starting another transition inside the check.
4. Keep prerequisite setup and transitions fail-fast. `verify` collects only
   independent returned check failures, reporting the checkpoint and every failed
   probe. It does not swallow panics or manufacture success after broken setup.
5. Run that existing exact journey, then the relevant owner and compatibility
   lanes. A new probe requires no extra provider script or host construction
   unless its observable behavior genuinely changes.

`tests/support/probes.rs` is a small root test helper with no global registry or
scenario engine. `tests/scenario_probes.rs` proves labeled multi-failure reporting and refuses
empty or duplicate probe sets. This keeps expensive execution shared while each
related guarantee remains individually identifiable.

## Hosted fake-agent permissions

`agent-permissions` uses the existing in-memory FakeProvider with strict scripted
turns, exact send/permission observations and a dedicated current-thread runtime.
It pumps actual host wake/tick events until concrete dock/provider predicates hold.
Scenario idleness alone does not account for Sessions' detached adapter jobs.
Runtime teardown must close the provider subscriber.

The fake shell step exercises a permission request and synthetic tool result; it
does not execute cargo, prove a file change or establish successful local review.
The scratch working file, recipe directory and review directory remain unchanged.
Notifications and real adapters are disabled. Production settings still perform
read-only PATH/current-directory lookups, so this is not a claim of zero ambient
reads. No credential lookup, command execution or MCP listener is requested.

SQLite evidence records actual host dock dispatches. Sessions currently supplies
no semantic turn/permission journal report, so a successful dispatch is not
labeled as a successful provider turn. Permission decisions remain in the real
dock/provider event history; metadata cache writes are not proof of durable
permission-history persistence across application restart.

## Direct selected-ticket setup

For a downstream Jira selection rule, the owner-local fixture can omit refresh
setup entirely:

```rust
let (plugin, fixture) = TicketSelected::new(
    TicketBuilder::new().summary("Find the missing dragon"),
).build(vec![])?;
```

`with_ticket` adds or replaces by stable ticket key, and `select(site, key)` names
an exact selection or returns an error. Real owner tree/fold/selection logic
chooses the row; no synthetic success message or action history is inserted.
Use `hosted_plugin` instead when the behavior under test is refresh itself.

The equivalence contract executes real identity, list and development requests,
then selects through Host intents and compares the actual neutral view and domain
state. It separately asserts that only the executed path has sync/journal evidence.
Six cases protect this distinction, direct downstream reuse, exact replacement,
invalid selections and strict injection across configuration/cache lifecycle.

## Local-review refusal checkpoint

`review-no-worktree` replays refresh and selection through GitLab and Sessions,
with no worktree-providing part installed. Its named starting state executes real
transitions. The detail response is held while probes verify request acceptance
and the nonterminal detail read. After release, Host produces its actual missing
worktree-provider refusal. Related probes share that execution to check the
visible refusal, exact provider requests, absent agent turns and saved reviews,
unchanged scratch directories, persisted parent/action evidence and closed
subscriber after runtime teardown.

The journal records successful request acceptance and detail dispatch. Those
facts do not assert completed review: there is no semantic review-success event.
This is a failure-path contract; successful worktree creation and review still
need a separately isolated Git write boundary.

## Protect a checkpoint's rendering cost

`Scenario::costs()` returns an owned snapshot. The `shell_frames` field counts
actual Ratatui shell renders on that scenario's Host, including raw `to_text`,
`to_svg`, `to_styles` and renders before the Host was adopted. A neutral-view
inspection does not render a frame. The GitHub checkpoint asserts one frame;
the other root journeys assert zero before closing their journal. The migrated
GitLab confirmation contract similarly asserts its one necessary pending frame.

```rust
assert_eq!(scenario.costs().shell_frames, 0, "selected-ticket rules need no frame");
```

The remaining fields (`dispatched_events`, `returned_effect_envelopes`,
`spawned_completion_tasks`) describe cumulative work routed through this driver.
They do not count detached Sessions jobs, Git subprocesses or all work inside a
future. Scheduling-dependent pump totals should not become arbitrary exact limits.
Choose a measured, purposeful bound only for a stable boundary. These counters
also do not count other Hosts, Ink renders or part-only buffer draws; use exact
provider ledgers and domain probes alongside them. Test discovery validates tier
inventory, not these runtime assertions or global construction costs.

## Accepted post with a lost response

Run `scripts/scenarios slack-lost-response` to inspect acceptance that the client
never observed. The scripted provider records acceptance only after matching the
exact write; the client receives a transport error and retains Unknown. A fresh
Host restores the captured serialized cache, requires fresh authentication, and
reads the accepted post without resending it. The first journal stays unchanged;
the second journal records the fresh read. This exercises the cache consumer,
not the production cache database, process restart or same-journal recovery.

## Diagnose renderer checkpoints cheaply

`scripts/check-scenario-renderers --preflight-only` checks the fixed GitHub
checkpoint inputs without Cargo or Ink. The full command then runs the real
producer and Ink replay, retaining named stage errors and bounded text differences
in a fresh outside-repo artifact directory. See
[the diagnostic workflow and evidence limits](scenario-renderer-diagnostics.md).
This does not add pixel/style checks or auto-accept golden output.
