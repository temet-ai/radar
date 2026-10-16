# Connect your services

Back to the [user guide](README.md).

Every service has a **connection** section in Settings (press `,`). They all look alike:

| Row | What it is |
|---|---|
| (found) | a question, if radar found a token in your environment |
| server, site or workspace | where the service is |
| token | `env:NAME` to read an environment variable each time, or the token itself |
| signed in as | who the service says the token belongs to, from the last check |
| check | what the last check said. `⏎` runs it now |
| refresh | how often radar asks the service for changes |

Move to a row and press `⏎` to change it. While you type, the right pane says what the
field accepts and whether what you typed is valid. After you change a connection, radar offers
to check it: it asks the service who you are and tells you whether it works.

**Where tokens are kept.** `env:NAME` stores only the name. A token you type or paste is
saved in the config file (`radar config path` says where) and shown masked.
Prefer `env:NAME`: then no secret sits in a file.

**In setup or in Settings.** Setup's step for a service (`radar setup gitlab`, `github`,
`jira`, `slack`) shows this same connection section, so both edit the same fields. Beside it,
setup shows help for the selected row and how to get a token, with links: `o` opens the
token page, `y` copies a command. A token radar found in your environment shows as a question
("Found: … Use it?"); in setup `space` takes it, because `⏎` moves on.

![Settings, a connection section](images/settings.gitlab.connection.png)

## GitLab

1. In GitLab, create a personal access token. radar asks for the `read_api` scope.
2. Export it, for example `export GITLAB_TOKEN=glpat-...`. `env:GITLAB_TOKEN` is the default.
   You can instead enter `glab` (radar reuses the login of the `glab` program; if GitLab turns
   that token down, radar asks `glab` to renew it once) or the token itself.
3. Settings, GitLab, connection: set the **server** (`https://gitlab.com` or your own).
   Optionally set a **base group**, so paths below can be short (for example `acme/platform`).
4. Settings, GitLab, **tenancies** (FOLLOW in setup's GitLab step): add the groups or
   repositories you follow. Paste a
   group's or repository's link, or its path. `+ add a tenancy` is always the first row.

Actions that change things (approve, merge, rebase, comment) go to GitLab with your token.
If GitLab refuses one, radar shows GitLab's reason.

## GitHub

GitHub support is **read only** and covers GitHub.com. It shows repositories, open pull
requests, the checks on a pull request's head commit, and Actions runs.

1. Sign in, one of three ways (Settings, GitHub, connection, or setup's GitHub step):
   * **public only**: no token; radar reads public repositories.
   * **gh's sign-in**: `gh` as the token. radar asks the `gh` program (`gh auth token`) when it
     needs the token, keeps it in memory for a minute at most, and never stores it.
   * **a token** that can read your private repositories, as `env:NAME` or pasted.
2. Settings, GitHub, **repositories**: create a group by entering `github.com/owner` (an
   owner's repositories) or a plain name such as `Client work` (your own group). With a token
   set, a picker opens so you can tick repositories. You can also enter `github.com/owner/repo`
   or `owner/repo` directly.

radar tracks at most 200 chosen repositories. It reads on start and when you press `r`; it does
not poll in the background.

![Settings, GitHub connection: the three ways to sign in, the repositories, and how to get a token](images/settings.github.connection.png)

## Jira

You need your Jira site, the email of your Atlassian account and an Atlassian API token
(created in your Atlassian account settings).

1. Settings, Jira, connection: give the **site** (`https://acme.atlassian.net`, or any link
   on the site), the **email** and the **token**. Each can be `env:NAME`. If radar finds
   variables such as `JIRA_BASE_URL`, `JIRA_EMAIL` and `JIRA_TOKEN`, it asks first.
2. Settings, Jira, **projects**: the project keys you follow, under each site. `p` picks from
   the projects you looked at lately on Jira.
3. Settings, Jira, **views**: each view is a saved query (JQL). The Jira screen shows one view at a time.

You can add more than one site. Each is one block in the connection section.

![Settings, Jira connection](images/settings.jira.connection.png)

## Slack

radar reads Slack as you, so it needs a **user token** (it starts with `xoxp-`). A bot token
does not do. The Slack app the token comes from needs these user scopes, depending on what
you use:

| What | Scopes |
|---|---|
| read the public channels you watch | `channels:history`, `channels:read` |
| read the private channels you watch | `groups:history`, `groups:read` |
| mentions of you, messages to you | `search:read` |
| who wrote what | `users:read` |
| your user groups | `usergroups:read` |
| post review asks and replies (after you say yes) | `chat:write` |
| add reactions (after you say yes) | `reactions:write` |
| pictures in threads | `files:read` |

After a check, radar tells you which of the scopes above the token lacks. Direct messages and
group messages that mention you come through `search:read`; radar asks for no `im:` or `mpim:`
scope.

**The quickest way to an app with those scopes** is radar's own app manifest. In setup's Slack
step (or Settings, Slack, connection), `m` copies the manifest and `o` opens Slack's new-app
page with it filled in. It asks for user scopes only. Install the app to your workspace and
copy its user token (`xoxp-…`).

1. Settings, Slack, connection: set the token as `env:NAME` or paste it.
2. Settings, Slack, **channels**: the channels to watch. Paste a channel's link, a link to a
   message in it, or its id.

![Settings, Slack connection: the token, what it may do, and how to make a token](images/settings.slack.connection.png)

