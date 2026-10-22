# radar documentation

Radar is a terminal workspace for GitLab, GitHub, Jira, Slack and local Git, with
coding agents beside your work. Start with the [project overview](../README.md)
for screenshots and capabilities.

These docs follow the source revision you are reading. `radar --version` tells
you which release and commit you installed; choose that tag on GitHub for matching
docs. Development on `main` can be ahead of published packages.

## Use radar

- [Install and upgrade](user-guide/install.md): supported platforms, Homebrew,
  mise, verified archives, PATH, updates and rollback.
- [Get started](user-guide/quickstart.md): a first session using local Git, with
  no service account needed.
- [User guide](user-guide/README.md): setup, navigation, modes and the CLI.
- [Connect services](user-guide/connect.md): credentials and required access.
- [Find a page](user-guide/pages.md): every screen and detail tab.
- [Keys](user-guide/keys.md) and [Settings](user-guide/settings.md): references.
- [Agents](user-guide/agents.md) and [MCP and local reviews](user-guide/mcp-and-reviews.md).
- [Troubleshooting](user-guide/troubleshooting.md) and [privacy](user-guide/privacy.md).

[Development changes since alpha.13](development-changes.md) summarizes the current
source transition and links the complete commit comparison.

## Develop radar

- [Contributing](../CONTRIBUTING.md): prerequisites, build, tests and changes.
- [Repository map](../AGENTS.md): where each part lives and working rules.
- [Builder's guide](GUIDE.md): architecture, UX contract and adding a part.
- [Components](components.md), [Ink renderer](../ink/README.md) and
  [IPC protocol](ink-ipc.md): view data and rendering.
- [Testing tiers](test-tiers.md), [scenarios](scenarios.md) and
  [renderer diagnostics](scenario-renderer-diagnostics.md).
- [Slack cache](slack-cache.md): persistence and recovery design.
- [Release guide](RELEASING.md): packaging, verification and distribution.

## Help and project policies

[Report a problem](user-guide/troubleshooting.md#report-a-problem), read the
[security policy](../SECURITY.md) or [code of conduct](../CODE_OF_CONDUCT.md), and
check the [release notes](https://github.com/temet-ai/radar/releases).
