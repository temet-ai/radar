#!/usr/bin/env bash
# capture-all.sh [OUT_DIR]
# Runs the real radar through the Ink renderer (the radar-ink-live skill's ink-live, the fake
# agent) and writes every user-guide screenshot as a PNG (default OUT_DIR: ../images).
# Needs the default-features release build (ink-live builds it), node, tmux, python3, Chrome
# or Chromium and ImageMagick.
#
# GitLab, Jira, GitHub and Slack are one local stand-in, fake-services.py, started here on a
# free port and stopped at the end; guide-config.toml points radar at it with dummy tokens.
# Nothing leaves the machine and nothing is written to any service. The stand-in's log of
# requests it did not know is kept in $work and shown at the end.
# KEEP_ANS=<dir> keeps the ANSI captures; KEEP_FAKE_LOG=<file> keeps the stand-in's stderr.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$here/../images}
mkdir -p "$out"; out=$(cd "$out" && pwd)
cols=160 rows=40
. "$here/lib.sh"

echo "splash"
wait_for_splash 5
sleep 1   # the beam has locked its contacts and the mark is in
shot splash
wait_for "Repositories" 20
wait_for "synced" 20

echo "repositories"
"$L" keys "$N" R
wait_for "team-a" 10
shot repositories
"$L" resize "$N" 100x30; sleep 1
shot repositories-100x30 100 30
"$L" resize "$N" "${cols}x$rows"; sleep 1

echo "menu"
click_on '!263' right
wait_for "ON GITLAB" 5
shot menu-row-actions
esc

echo "merge request"
click_on '!263'
"$L" keys "$N" Enter
wait_for "Overview" 10
shot detail-merge-request
esc; esc

echo "agent dock"
"$L" keys "$N" R
click_on '!263'
"$L" keys "$N" A
wait_for "agents" 10
sleep 2
# the composer has the keys: a message, and the fake agent's reply
tm send-keys -t radar -l "Please summarise this merge request"
sleep 0.3
tm send-keys -t radar Enter
wait_for "You said" 10
sleep 1
shot agent-dock
"$L" keys "$N" M-w   # alt-w closes the dock's last tab, which hides the dock
esc

echo "github"
"$L" keys "$N" R
click_on 'GitHub'
wait_for "PRs" 20
"$L" keys "$N" Down
shot github

echo "local workspaces"
"$L" keys "$N" R
click_on 'Local workspaces'
"$L" keys "$N" Down r
wait_for "  api" 10
for _ in $(seq 1 20); do text | grep -qF "looking for notes" || break; sleep 0.25; done
"$L" keys "$N" Down
shot local-workspaces

echo "jira"
"$L" keys "$N" J
wait_for "ABC-12" 10
shot jira
click_on 'ABC-12'
"$L" keys "$N" Enter
wait_for "ABC-12" 10
sleep 1
shot detail-jira-issue
esc; esc

echo "slack, branches, work, history"
"$L" keys "$N" S; wait_for "Slack" 5; shot slack
click_on '#team  '   # the channel in the left list (two spaces: the other #team words have one)
wait_for "Lunch at 12:30" 10
shot slack-channel
"$L" keys "$N" B; wait_for "CLONES" 10; shot branches

# Work: three tasks and a group, written the way `radar work apply` does (the screen's own `n`
# cannot save in the read-only mode ink-live runs radar in). Same journal as the running radar.
gw() {
  env -u RADAR_NO_WRITES HOME="$d/home" XDG_CONFIG_HOME="$d/cfg" XDG_STATE_HOME="$d/state" \
    XDG_CACHE_HOME="$d/cache" XDG_DATA_HOME="$d/data" RADAR_MODE=edit "$d/bin/radar" work "$@"
}
change() { # MUTATION-JSON
  printf '{"change_id":"%s","basis":"user_asserted","evidence":{"type":"assertion"},"mutation":%s}' "$(gw id)" "$1"
}
group=$(gw id)
group_change=$(change "{\"type\":\"create_tag\",\"tag_id\":\"$group\",\"namespace\":\"work.group\",\"key\":\"$group\",\"label\":\"This week\"}")
task() { # TITLE [grouped]
  local item tag changes
  item=$(gw id); tag=$(gw id)
  changes=$(change "{\"type\":\"create_item\",\"item_id\":\"$item\",\"kind\":\"task\"}")
  changes="$changes,$(change "{\"type\":\"create_tag\",\"tag_id\":\"$tag\",\"namespace\":\"work.title\",\"key\":\"$item\",\"label\":\"$1\"}")"
  changes="$changes,$(change "{\"type\":\"assign_tag\",\"tag_id\":\"$tag\",\"target\":{\"type\":\"item\",\"id\":\"$item\"}}")"
  if [ -n "${2:-}" ]; then
    changes="$changes,$(change "{\"type\":\"assign_tag\",\"tag_id\":\"$group\",\"target\":{\"type\":\"item\",\"id\":\"$item\"}}")"
  fi
  printf '{"version":1,"changes":[%s]}' "$changes" | gw apply --stdin >/dev/null
}
printf '{"version":1,"changes":[%s]}' "$group_change" | gw apply --stdin >/dev/null
task "Review the stats endpoint before Friday" grouped
task "Answer Bob about p95 latency" grouped
task "Draft the release notes for 1.4"
"$L" keys "$N" I; wait_for "work items" 5
"$L" keys "$N" r; wait_for "Review the stats endpoint" 10; sleep 1
shot work
"$L" keys "$N" H; wait_for "History" 5; sleep 1; shot history

echo "settings"
esc
"$L" keys "$N" ,
wait_for "General" 5
# the settings nav lists one row per line from screen line 3; General is row 0
nav() { "$L" click "$N" 12 $((3 + $1)); sleep 0.5; }
nav 1;  wait_for "parts" 3;  shot settings.general.parts
nav 6;  wait_for "GitLab connection" 3; shot settings.gitlab.connection
nav 11; wait_for "GitHub connection" 3; shot settings.github.connection
nav 14; wait_for "Jira connection" 3; shot settings.jira.connection
nav 19; wait_for "Slack connection" 3; shot settings.slack.connection
nav 32; wait_for "Tools and defaults" 3; shot settings.sessions.agents

if grep -E '^(UNKNOWN|ERROR|graphql: no field)' "$work/fake.err" >"$work/gaps" 2>/dev/null && [ -s "$work/gaps" ]; then
  echo "capture-all: the stand-in met requests or fields it does not cover:" >&2
  sort "$work/gaps" | uniq -c | sort -rn | head -n 30 >&2
fi
echo "done: $out"
