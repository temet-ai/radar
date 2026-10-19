# History: follow recorded actions

History shows radar's local record of operations: reads, checks, launches, and writes
you confirmed or declined. It helps you trace what happened to a selected item without
leaving the workspace. These are recorded observations, kept in the
[local activity store](privacy.md#files).

Press `H` to open the last 24 hours, or choose **history for selected item** from an
item's `x` menu to open only that exact source's events. History opens over your current
screen; `esc` returns to it.

![History with event rows and a selected event's detail](images/history.png)

This [example capture](images/INDEX.md) shows a demo session's startup reads. The build,
timestamps and folder names are from that capture, not your current session.

## Check what happened to one operation

Suppose you started local work for the fictional `WATCH-12` ticket from
[Workspaces](workspaces.md#start-a-ticket-in-several-repositories), and want to check
which repositories started successfully.

1. Select the ticket and open **history for selected item** from `x`.
2. Use `↑`/`↓` (or `j`/`k`) to choose its start-work event. History lists original events
   in recording order, oldest first; press `n` for the next page if needed.
3. Press `⏎` to focus the event detail. Read the outcome, reason, subjects and any
   recorded duration. `PgUp`/`PgDn` scroll the detail from either pane.
4. Press `c` to show events sharing that event's exact causal root. This is the way to
   follow the recorded operation and its child actions. Select a repository result to
   see its own outcome.
5. Press `i` to narrow further to the selected exact action ID. Press `a` for all
   activity, or `t` to return to the last 24 hours.

Use the recorded result to decide what needs investigation. A **requested**, **confirmed**
or **started** event is not a successful result. A partial or unknown result needs its
related events inspected; it should not be treated as complete.

## Event list and detail

The event list has **observed UTC**, **action**, **subject** and **outcome** columns.
An action can have several events as it progresses. The detail gives the selected event's
observed and recorded times, outcome and recorded metadata such as reason, mode, count,
duration, command or safe URL when present.

Below those fields are the exact recorded subjects and the source of the recording:
part or host, TUI/CLI session, build version, action/event/session IDs, parent and root
IDs, correlation ID and sequence. A field labelled **not recorded** means that evidence
is absent, not that the action had no reason or restriction. Redaction, truncation,
omitted-subject and estimated-time notices are displayed when applicable.

On a wide terminal, events and detail sit side by side. On a narrow terminal, change
focus to see the other pane. `tab` switches focus; `←` focuses events and `→` or `⏎`
focuses detail. `Home`/`End` move to the first/last event or the top/bottom of focused detail.

### Scopes and pages

| Key or entry point | Scope |
|---|---|
| `H` / `t` | Recent activity, the last 24 hours |
| An item's **history for selected item** | Events with that exact scoped source identity |
| `c` | Events with the selected event's exact causal root |
| `i` | Events for the selected exact action ID |
| `a` | All recorded activity |
| `n` / `p` | Next / previous page within the current scope and snapshot |
| `r` | A fresh snapshot of the scope, starting at its first page |

Each page contains at most 50 events. The header says **snapshot through #…**: paging
keeps that boundary, so newly arriving events do not shift the pages underneath you.
Refresh to include newer records. Refreshing recent activity also advances its 24-hour
window. Times are UTC; the display order follows recording sequence, so it can differ
from timestamp order when clocks differ.

`x` or `?` opens the History key reference. While it is open, scroll the reference with
the usual scroll keys. `esc` closes the reference first, then closes History.

## Empty or unavailable history

| Message | Meaning and next step |
|---|---|
| Loading recorded observations | A local read is pending |
| No recorded events in this 24-hour window | Press `a` for older activity or `r` for a fresh read |
| No recorded events in this scope | Broaden with `a`, or return to the source and choose another item |
| History could not be read | Press `r` to retry, or `esc` to return to your work |
| Recording disabled or unavailable for this session | Check [local storage](privacy.md#files) and its access; close History to continue |
| Recording gaps or incomplete coverage | Some observations may be missing; absence of an event does not prove an operation never happened |

History is read only in every radar mode. It does not replay an operation or send a
message. Parent and root IDs establish the recorded causal links; sharing a ticket,
repository or [Work group](work.md) alone does not show that two events were part of
the same operation.

For connection and runtime problems behind a recorded failure, continue with
[When something looks wrong](troubleshooting.md).
