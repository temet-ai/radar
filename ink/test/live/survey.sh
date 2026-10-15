#!/usr/bin/env bash
# survey.sh OUTDIR SCENARIOS — runs the same keys on Ink and Ratatui (160x40, demo data)
# and prints the style diff of each step. SCENARIOS: one step per line, `name<TAB>keys`
# (tmux key names, space separated; `click:x:y:how` for a mouse step, `sleep:N`, `find:TEXT` clicks the row holding TEXT).
# Needs the radar-ink-live skill's script and a node on PATH. Ratatui is the reference.
set -euo pipefail
root=$(git rev-parse --show-toplevel); L=$root/scripts/ink-live/ink-live
out=$1; sc=$2; mkdir -p "$out"
key=$(printf '%s|%s' "$root" "${TMPDIR:-}" | cksum | cut -d ' ' -f 1)
cap() { tmux -L "radar-live-$key-$1" capture-pane -e -p -t radar; }
$L start sv_i ink 160x40 >/dev/null; $L start sv_r ratatui 160x40 >/dev/null
sleep "${SETTLE:-3}" # the first sync lands in both before the first key, or the two start from different states
trap '$L stop sv_i >/dev/null; $L stop sv_r >/dev/null' EXIT
while IFS=$'\t' read -r name keys; do
  [ -z "$name" ] && continue
  for r in i r; do
    for k in $keys; do
      case $k in
        click:*) IFS=: read -r _ x y how <<<"$k"; $L click sv_$r "$x" "$y" "${how:-left}" ;;
        sleep:*) sleep "${k#sleep:}" ;;
        find:*) # click the first screen row whose text holds the words (underscores stand for spaces)
          y=$($L text sv_$r | grep -n -m1 -F -- "${k#find:}" | cut -d: -f1 | head -n1 || true)
          [ -n "$y" ] && $L click sv_$r 30 "$y" left ;;
        *) $L keys sv_$r "$k" ;;
      esac
    done
  done
  cap sv_i > "$out/$name.ink.ansi"; cap sv_r > "$out/$name.rat.ansi"
  echo "=== $name ($keys)"; "${NODE:-node}" "$root/ink/test/live/style-diff.mjs" "$out/$name.ink.ansi" "$out/$name.rat.ansi" | tail -n +1
done < "$sc"
