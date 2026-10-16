# lib.sh: what capture-all.sh and clip.sh share. Sourced, not run.
# Before sourcing, set `out` (where pictures go); `cols` and `rows` may be set (default 160x40).
# It starts the stand-in services and radar (Ink, the fake agent, ink-live's read-only mode),
# gives the Git screens clones to find, and defines the helpers below. The caller's EXIT trap
# is `cleanup`.

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(git -C "$here" rev-parse --show-toplevel)
L=$root/scripts/ink-live/ink-live
N=${GUIDE_RUN:-gd}
cols=${cols:-160} rows=${rows:-40}
work=$(mktemp -d "${TMPDIR:-/tmp}/radar-guide.XXXXXX")
fake_pid=
cleanup() {
  "$L" stop "$N" >/dev/null 2>&1 || true
  if [ -n "$fake_pid" ]; then kill "$fake_pid" >/dev/null 2>&1 || true; fi
  if [ -n "${KEEP_FAKE_LOG:-}" ] && [ -f "$work/fake.err" ]; then cp "$work/fake.err" "$KEEP_FAKE_LOG"; fi
  rm -rf "$work"
}
trap cleanup EXIT
# Nothing from the machine's own accounts may reach a screen (radar offers tokens it finds in the environment).
# Anywhere in the name: PROJECT_GITLAB_TOKEN is offered too.
for v in $(env | cut -d= -f1 | grep -E '(SLACK|GITHUB|GITLAB|GLAB|JIRA|ATLASSIAN)|^GH_' || true); do unset "$v"; done

# The stand-in for the four services, and the config that points radar at it.
# A fixed port keeps the address on the screens the same from run to run; a busy one falls back to any free port.
start_fake() { # PORT
  python3 -I "$here/fake-services.py" --port "$1" >"$work/fake.out" 2>"$work/fake.err" &
  fake_pid=$!
  fake=
  for _ in $(seq 1 30); do
    fake=$(head -n 1 "$work/fake.out" 2>/dev/null || true)
    if [ -n "$fake" ]; then return 0; fi
    kill -0 "$fake_pid" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}
start_fake "${GUIDE_FAKE_PORT:-38080}" || start_fake 0 || true
[ -n "$fake" ] || { echo "guide tools: fake-services.py did not start:" >&2; cat "$work/fake.err" >&2; exit 1; }
echo "fake services on $fake"
sed "s|@FAKE@|$fake|g" "$here/guide-config.toml" >"$work/config.toml"
export RADAR_LIVE_CONFIG=$work/config.toml RADAR_LIVE_OFFLINE=1 RADAR_FAKE_AGENT=1

text() { "$L" text "$N"; }
shot() { # NAME [COLS ROWS]: capture the screen now, render it to NAME.png
  local c=${2:-$cols} r=${3:-$rows}
  "$L" colour "$N" >"$work/$1.ans"
  "$here/render.sh" "$work/$1.ans" "$out/$1.png" "$c" "$r"
  if [ -n "${KEEP_ANS:-}" ]; then mkdir -p "$KEEP_ANS"; cp "$work/$1.ans" "$KEEP_ANS/"; fi
  echo "  $1.png"
}
wait_for() { # PATTERN [SECONDS]: poll the screen text for a fixed string
  local i
  for i in $(seq 1 $((${2:-10} * 4))); do
    if text | grep -qF -- "$1"; then return 0; fi
    sleep 0.25
  done
  echo "guide tools: never saw '$1'; the screen is:" >&2; text >&2; return 1
}
# The splash's scope is braille (U+2801..U+28FF); wait until it is drawn.
wait_for_splash() { # [SECONDS]
  local i
  for i in $(seq 1 $((${1:-5} * 10))); do
    if text | python3 -I -c 'import sys; sys.exit(0 if any("⠁" <= c <= "⣿" for c in sys.stdin.read()) else 1)'; then return 0; fi
    sleep 0.1
  done
  echo "guide tools: the splash never showed" >&2; return 1
}
row_of() { text | grep -nF -- "$1" | head -n 1 | cut -d: -f1; }
# Characters, not bytes: macOS awk's index() counts bytes, and icons are several each.
col_of() { text | grep -F -- "$1" | head -n 1 | python3 -I -c 'import sys; p = sys.argv[1]; print(sys.stdin.read().index(p) + 1)' "$1"; }
click_on() { # TEXT [left|right]: click just inside the first occurrence of TEXT on screen
  local y x; y=$(row_of "$1"); x=$(col_of "$1")
  [ -n "$y" ] || { echo "guide tools: '$1' not on screen" >&2; exit 1; }
  "$L" click "$N" $((x + 1)) "$y" "${2:-left}"
}
esc() { "$L" keys "$N" Escape; "$L" keys "$N" Escape; }

# --- start, then give the Git screens clones to find ----------------------------------
d=$("$L" start "$N" ink "${cols}x$rows" | tee /dev/stderr | sed -n 's/.*files in //p')
mkdir -p "$d/home/code/repositories"
"$here/make-fixtures.sh" "$work/fx" "$fake"
cp -R "$work/fx/." "$d/home/code/repositories/"
key=$(printf '%s|%s' "$root" "${TMPDIR:-}" | cksum | cut -d ' ' -f 1)
tm() { tmux -L "radar-live-$key-$N" "$@"; }
# radar scans for clones at start: run it again (the same command, same data) to see them.
# A caller that records the start sets GUIDE_NO_RESTART=1 and restarts it itself (`restart`).
restart() { tm respawn-pane -k -t radar; }
[ -n "${GUIDE_NO_RESTART:-}" ] || restart
