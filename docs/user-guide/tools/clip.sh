#!/usr/bin/env bash
# clip.sh NAME [OUT_DIR]: record one short clip of the real radar for the docs.
# NAME is one of the stories below (demo). Writes OUT_DIR/NAME.gif (for the README, which
# GitHub plays inline) and OUT_DIR/NAME.mp4 (default OUT_DIR: ../images).
# Same stand-in data and needs as capture-all.sh, plus ffmpeg. Each screen that changed is
# grabbed (frames.py, 10 a second), drawn as capture-all draws a still (render.sh) and
# shown for as long as it was on screen.
# CLIP_COLS / CLIP_ROWS set the terminal (default 120x36); CLIP_WIDTH the clip's width in
# pixels (default: the terminal's own, no scaling).
set -euo pipefail

name=${1:?usage: clip.sh NAME [OUT_DIR]}
here=$(cd "$(dirname "$0")" && pwd)
out=${2:-$here/../images}
mkdir -p "$out"; out=$(cd "$out" && pwd)
cols=${CLIP_COLS:-120} rows=${CLIP_ROWS:-36}   # the splash's scope needs 34 rows
command -v ffmpeg >/dev/null || { echo "clip.sh: needs ffmpeg" >&2; exit 1; }
GUIDE_NO_RESTART=1 . "$here/lib.sh"

frames=$work/frames
rec_pid=
# The grabber stops with the script, whichever way it ends.
trap '[ -z "$rec_pid" ] || kill "$rec_pid" 2>/dev/null || true; cleanup' EXIT
record() { python3 -I "$here/frames.py" "$frames" 10 "$L" colour "$N" & rec_pid=$!; }
finish() { touch "$frames/stop"; wait "$rec_pid"; rec_pid=; }
pause() { sleep "${1:-0.8}"; }
type_in() { tm send-keys -t radar -l "$1"; }

# --- the stories -----------------------------------------------------------------------
demo() { # the README's: start, the lists, a merge request's page, Jira, Slack, an agent
  record
  restart
  wait_for "synced" 20; pause 1
  "$L" keys "$N" R; wait_for "team-a" 10; pause 1
  "$L" keys "$N" Down; pause 0.6; "$L" keys "$N" Down; pause 1
  click_on '!263'; "$L" keys "$N" Enter; wait_for "Overview" 10; pause 1.5
  "$L" keys "$N" ']'; pause 1.5
  esc; pause 0.5
  "$L" keys "$N" J; wait_for "ABC-12" 10; pause 1.5
  "$L" keys "$N" S; wait_for "SLACK" 5; pause 1.5
  "$L" keys "$N" R; wait_for "team-a" 10; pause 0.5
  click_on '!263'; "$L" keys "$N" A; wait_for "agents" 10; pause 1
  type_in "What is left before this can merge?"; pause 0.5
  tm send-keys -t radar Enter; wait_for "You said" 10; pause 2
  finish
}

case $name in
  demo) demo ;;
  *) echo "clip.sh: no story called $name (demo)" >&2; exit 2 ;;
esac

# --- frames to a clip ------------------------------------------------------------------
n=$(($(wc -l <"$frames/times.txt") - 1))
[ "$n" -gt 0 ] || { echo "clip.sh: no frames were grabbed" >&2; exit 1; }
echo "rendering $n frames"
list=$work/list.txt; : >"$list"
for i in $(seq 0 $((n - 1))); do
  f=$(printf '%04d' "$i")
  "$here/render.sh" "$frames/$f.ans" "$frames/$f.png" "$cols" "$rows"
  dur=$(awk -v i="$i" 'NR == i + 1 { a = $1 } NR == i + 2 { b = $1 } END { printf "%.3f", b - a }' "$frames/times.txt")
  printf "file '%s'\nduration %s\n" "$frames/$f.png" "$dur" >>"$list"
done
# The concat demuxer takes the last frame's duration only when it is listed again.
printf "file '%s'\n" "$frames/$(printf '%04d' $((n - 1))).png" >>"$list"
scale=${CLIP_WIDTH:+scale=$CLIP_WIDTH:-1:flags=lanczos,}
ffmpeg -v error -y -f concat -safe 0 -i "$list" \
  -vf "${scale}fps=10,split[a][b];[a]palettegen=max_colors=128:stats_mode=full[p];[b][p]paletteuse=dither=none" \
  -loop 0 "$out/$name.gif"
ffmpeg -v error -y -f concat -safe 0 -i "$list" \
  -vf "${scale}fps=25,pad=ceil(iw/2)*2:ceil(ih/2)*2,format=yuv420p" -c:v libx264 -crf 20 -movflags +faststart \
  "$out/$name.mp4"
ls -l "$out/$name.gif" "$out/$name.mp4"
