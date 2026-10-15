#!/usr/bin/env bash
# render.sh CAPTURE.ans OUT.png [COLS ROWS]
# `ink-live colour` capture -> terminal-looking PNG (ANSI -> HTML on a cell grid -> headless Chrome).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
in=$1 out=$2 cols=${3:-160} rows=${4:-40}
chrome=$(command -v google-chrome-stable || command -v google-chrome || command -v chromium \
  || { [ -x "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ] && echo "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"; } || true)
[ -n "$chrome" ] || { echo "render.sh: no Chrome found" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
CW=${CELL_W:-9.92}; CHH=20
# 0xProto Nerd Font Mono by its files, not its name (a fontconfig alias may map
# the name to another font, whose advance does not fit the 9.92px cell).
face() { # STYLE: the file of that 0xProto Nerd Font Mono style, from fontconfig, GUIDE_FONT_DIR or the usual folders
  local f=""
  if command -v fc-list >/dev/null; then
    f=$(fc-list "0xProto Nerd Font Mono:style=$1" file 2>/dev/null | head -n 1 | sed 's/:[[:space:]]*$//')
  fi
  if [ -z "$f" ]; then
    for d in ${GUIDE_FONT_DIR:-} "$HOME/Library/Fonts" /Library/Fonts "$HOME/.local/share/fonts" /usr/share/fonts/TTF; do
      [ -f "$d/0xProtoNerdFontMono-$1.ttf" ] && { f="$d/0xProtoNerdFontMono-$1.ttf"; break; }
    done
  fi
  echo "$f"
}
export FONT_REGULAR FONT_BOLD FONT_ITALIC
FONT_REGULAR=$(face Regular) FONT_BOLD=$(face Bold) FONT_ITALIC=$(face Italic)
[ -n "$FONT_REGULAR" ] || echo "render.sh: 0xProto Nerd Font Mono not found; the cells may not line up" >&2
python3 -I "$here/ansi2html.py" "$cols" "$rows" "$CW" <"$in" >"$tmp/p.html"
w=$(awk -v c="$cols" -v w="$CW" 'BEGIN{printf "%d", c*w+0.5}'); h=$((rows*CHH)); sc=${SCALE:-1}; cw_px=$((w*sc)); ch_px=$((h*sc))
"$chrome" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=${SCALE:-1} \
  --window-size="$w,$((h+200))" --screenshot="$tmp/full.png" "file://$tmp/p.html" >/dev/null 2>&1
if command -v magick >/dev/null; then magick "$tmp/full.png" -crop "${cw_px}x${ch_px}+0+0" +repage "$out"
elif command -v convert >/dev/null; then convert "$tmp/full.png" -crop "${cw_px}x${ch_px}+0+0" +repage "$out"
else cp "$tmp/full.png" "$out"; fi
