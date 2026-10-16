# Security policy

radar runs on your machine with your tokens for GitLab, GitHub, Jira and Slack, and it
starts the coding agents you have installed. A flaw that leaks a token, writes to a service
without your say, or runs something you did not ask for is a security problem, and we want
to hear about it.

## Reporting a vulnerability

Report it privately through GitHub: on [temet-ai/radar](https://github.com/temet-ai/radar),
open the **Security** tab and choose **Report a vulnerability**. Please do not open a public
issue for it.

Say what you found, how to reproduce it, and which version (`radar --version`) and platform
you used. We aim to reply within a week. radar is in alpha and maintained by a small team, so
fixes ship in the next alpha release rather than as patches to older ones.

## Supported versions

Only the latest alpha release is supported.

## What radar does with your credentials

Tokens are read from the places your config names (an environment variable, another tool's
login such as `glab`, or the config file itself) and are sent only to the service they
belong to, at the address you configured. radar never sends them anywhere else and never
logs them. The coding agents run as your own `claude`, `codex` or `copilot` programs with
their own logins, which radar does not read or store.
