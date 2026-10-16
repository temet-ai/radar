# Security policy

radar runs on your machine with your tokens for GitLab, GitHub, Jira and Slack, and it
starts the coding agents you have installed. A flaw that leaks a token, writes to a service
without your confirmation, or runs something you did not ask for is a security problem.

## Reporting a vulnerability

Report it privately through GitHub: on [temet-ai/radar](https://github.com/temet-ai/radar),
open the **Security** tab and choose **Report a vulnerability**. Please do not open a public
issue for it.

Say what you found, how to reproduce it, and which version (`radar --version`) and platform
you used. We aim to reply within a week.

## Supported versions

The latest alpha release is supported. Fixes ship in the next alpha release, not as patches
to older ones.

## Credentials

[What radar stores and sends](docs/user-guide/privacy.md) lists where tokens are kept, where
they are sent and what radar writes to disk.
