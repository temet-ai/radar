#!/usr/bin/env bash
# make-fixtures.sh DIR [GITLAB_URL]
# A few small git checkouts under DIR (api, web, notes), with fixed dates, so the Git and
# Local workspaces screens have something to show. `api` and `web` carry the GitLab
# fixture's remote, so radar matches them to acme/team-a/api and acme/team-a/web.
set -euo pipefail
dir=$1
gl=${2:-https://gitlab.example}   # the GitLab address of the config (the stand-in's, in a capture)
rm -rf "$dir"; mkdir -p "$dir"
export GIT_AUTHOR_NAME="Alice Example" GIT_AUTHOR_EMAIL=alice@example.com
export GIT_COMMITTER_NAME="Alice Example" GIT_COMMITTER_EMAIL=alice@example.com
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
commit() { # repo, message, day
  GIT_AUTHOR_DATE="2026-10-$3T10:00:00Z" GIT_COMMITTER_DATE="2026-10-$3T10:00:00Z" git -C "$1" commit -q -am "$2"
}
mk() { # name remote
  local r=$dir/$1
  git init -q -b main "$r"
  [ -z "${2:-}" ] || git -C "$r" remote add origin "$2"
  printf 'one\n' >"$r/README.md"; git -C "$r" add .
  GIT_AUTHOR_DATE="2026-10-01T10:00:00Z" GIT_COMMITTER_DATE="2026-10-01T10:00:00Z" git -C "$r" commit -q -m "First commit"
}
mk api "$gl/acme/team-a/api.git"
git -C "$dir/api" checkout -q -b feature/abc-12-stats
printf 'stats\n' >"$dir/api/stats.rs"; git -C "$dir/api" add .; commit "$dir/api" "ABC-12: Add a stats endpoint" 06
printf 'dirty\n' >>"$dir/api/README.md"
git -C "$dir/api" stash -q 2>/dev/null || true
printf 'wip\n' >>"$dir/api/README.md"
mk web "$gl/acme/team-a/web.git"
git -C "$dir/web" branch -q fix/header
mk notes ""
printf 'todo\n' >"$dir/notes/todo.md"
