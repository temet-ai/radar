# Slack cache and recovery

Slack's JSON cache is versioned and partitioned by the authenticated workspace ID
and user ID. A cached identity is never a login. Each new TUI session (including a
configured token-reference change) authenticates before selecting its account's
messages, names, groups, read marks, opened threads and followed review asks.
`radar slack list` and `radar slack catchup` also authenticate on every invocation.

Changing accounts parks the previous account's data. Returning to the same
workspace **and user** restores that partition after successful authentication.
A different user in the same workspace starts with a separate partition. Failed
or incomplete authentication does not fall back to cached identity or marks.
There may therefore be no cached content on the first frame, or while offline.

## Older caches

Old `data` caches have no trustworthy ownership boundary and may already contain
entries from more than one account. They are retained as unverified JSON, even
when their saved identity matches the current login. They are not automatically
adopted, followed, used as caught-up marks, or used to create account-scoped links
and history. Unknown future versions and malformed envelopes are preserved as
whole JSON values too. JSON content is preserved, not original whitespace or key
ordering. Unknown fields are never silently dropped into a supported envelope.

To export the retained unverified values to a local file:

```sh
radar slack cache-recovery > slack-cache-recovery.json
```

This command requires no Slack credentials and makes no provider request. It
prints a JSON array (`[]` when nothing is quarantined), without changing or
adopting the cache. The export may contain private messages and kept work; store
it privately. There is no automatic import, replay, or mark-adoption command.
Fresh provider reads build a new verified partition without deleting the old
JSON. Existing cache content is preserved as supplied; radar does not add credential
values, token references or credential hashes as ownership keys.

## In-progress writing

An account/token transition closes old thread pages and makes pending choices
stale. Authored reply and review-ask text stays available within the running TUI.
A review ask from an earlier or unverified session cannot obtain a fresh send
confirmation under another account. Use `y` on its draft page or in Slack's
connection settings to copy the authored words, then deliberately start a new
draft for the intended account. Kept replies remain recoverable through the
existing kept-replies controls. These editors are not restart-persistent; the cache
export is not a backup of in-memory editors.

A write already dispatched can still finish after switching accounts. Its
original history and outcome are retained; a successful review ask is followed
only in its original account partition, never the newly selected account.
Nothing is automatically resent.

Configured channel aliases and review settings remain explicit configuration;
they are not evidence of cache ownership. radar does not rewrite existing
SQLite history or detect a secret changing behind an unchanged environment/token
reference midway through an authenticated session. Restart or recheck the
connection after such a change.
