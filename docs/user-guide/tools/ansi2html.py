#!/usr/bin/env python3
"""ansi2html.py COLS ROWS < capture.ans > page.html

Turns `ink-live colour` output (tmux capture-pane -e: SGR colours, one line per row)
into an HTML page where every cell sits on a fixed grid, so box-drawing, Nerd Font
icons and wide characters line up like a real terminal.
"""
import html, os, sys, re, unicodedata

FONT = "0xProto Nerd Font Mono"
FS = 16            # px
CW = 9.92          # cell width px (0xProto advance 0.6em); overridden by argv[3]
CH = 20            # cell height px
DEF_FG, DEF_BG = (248, 248, 242), (18, 16, 26)
BASE = [(0,0,0),(255,85,85),(80,250,123),(241,250,140),(130,170,255),(255,121,198),(139,233,253),(191,191,191),
        (98,114,164),(255,110,110),(105,255,148),(255,255,165),(155,190,255),(255,146,223),(164,255,255),(255,255,255)]

def xterm(n):
    if n < 16: return BASE[n]
    if n < 232:
        n -= 16; f = lambda v: 0 if v == 0 else 55 + 40 * v
        return (f(n // 36), f(n // 6 % 6), f(n % 6))
    v = 8 + 10 * (n - 232); return (v, v, v)

def width(ch):
    if unicodedata.combining(ch): return 0
    if unicodedata.east_asian_width(ch) in "WF": return 2
    o = ord(ch)
    if 0x1F300 <= o <= 0x1FAFF: return 2
    return 1

def parse(line):
    """yield (char, style) cells; style = (fg, bg, bold, dim, ital, under, rev, strike)"""
    st = dict(fg=None, bg=None, b=0, d=0, i=0, u=0, r=0, s=0)
    pos = 0
    for m in re.finditer(r"\x1b\[([0-9;:]*)m|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-9;?]*[A-Za-z]", line):
        yield from ((c, dict(st)) for c in line[pos:m.start()])
        pos = m.end()
        if not m.group(0).endswith("m") or m.group(1) is None: continue
        p = [int(x) if x else 0 for x in re.split(r"[;:]", m.group(1))] or [0]
        k = 0
        while k < len(p):
            c = p[k]
            if c == 0: st = dict(fg=None, bg=None, b=0, d=0, i=0, u=0, r=0, s=0)
            elif c == 1: st["b"] = 1
            elif c == 2: st["d"] = 1
            elif c == 3: st["i"] = 1
            elif c == 4: st["u"] = 1
            elif c == 7: st["r"] = 1
            elif c == 9: st["s"] = 1
            elif c == 22: st["b"] = st["d"] = 0
            elif c == 23: st["i"] = 0
            elif c == 24: st["u"] = 0
            elif c == 27: st["r"] = 0
            elif c == 29: st["s"] = 0
            elif 30 <= c <= 37: st["fg"] = xterm(c - 30)
            elif 90 <= c <= 97: st["fg"] = xterm(c - 90 + 8)
            elif 40 <= c <= 47: st["bg"] = xterm(c - 40)
            elif 100 <= c <= 107: st["bg"] = xterm(c - 100 + 8)
            elif c == 39: st["fg"] = None
            elif c == 49: st["bg"] = None
            elif c in (38, 48):
                key = "fg" if c == 38 else "bg"
                if k + 1 < len(p) and p[k+1] == 5 and k + 2 < len(p): st[key] = xterm(p[k+2]); k += 2
                elif k + 1 < len(p) and p[k+1] == 2 and k + 4 < len(p): st[key] = tuple(p[k+2:k+5]); k += 4
            k += 1
    yield from ((c, dict(st)) for c in line[pos:])

def css(st):
    fg = st["fg"] or DEF_FG; bg = st["bg"] or DEF_BG
    if st["r"]: fg, bg = bg, fg
    out = ["color:#%02x%02x%02x" % fg]
    if bg != DEF_BG: out.append("background:#%02x%02x%02x" % bg)
    if st["b"]: out.append("font-weight:700")
    if st["d"]: out.append("opacity:.6")
    if st["i"]: out.append("font-style:italic")
    dec = ("underline " if st["u"] else "") + ("line-through" if st["s"] else "")
    if dec: out.append("text-decoration:" + dec.strip())
    return ";".join(out)

def main():
    cols, rows = int(sys.argv[1]), int(sys.argv[2])
    cw = float(sys.argv[3]) if len(sys.argv) > 3 else CW
    lines = sys.stdin.read().split("\n")[:rows]
    lines += [""] * (rows - len(lines))
    body = []
    for ln in lines:
        cells = []     # (char, style, w)
        for ch, st in parse(ln.rstrip("\r")):
            w = width(ch)
            if w == 0 and cells: cells[-1] = (cells[-1][0] + ch, cells[-1][1], cells[-1][2]); continue
            cells.append((ch, st, w))
        used = sum(c[2] for c in cells)
        if used < cols: cells.append((" " * (cols - used), dict(fg=None, bg=None, b=0, d=0, i=0, u=0, r=0, s=0), cols - used))
        spans = []
        x = 0
        i = 0
        while i < len(cells):
            ch, st, w = cells[i]
            ascii_ok = all(ord(c) < 128 for c in ch) and w == len(ch)
            if ascii_ok:
                j = i; txt = ""
                while j < len(cells) and cells[j][1] == st and all(ord(c) < 128 for c in cells[j][0]) and cells[j][2] == len(cells[j][0]):
                    txt += cells[j][0]; j += 1
                spans.append('<span class="r" style="left:%.2fpx;width:%.2fpx;%s">%s</span>' % (x * cw, len(txt) * cw, css(st), html.escape(txt).replace(" ", "&nbsp;")))
                x += len(txt); i = j
            else:
                spans.append('<span class="c" style="left:%.2fpx;width:%.2fpx;%s">%s</span>' % (x * cw, w * cw, css(st), html.escape(ch)))
                x += w; i += 1
        body.append('<div class="l">' + "".join(spans) + "</div>")
    W, H = round(cols * cw), rows * CH
    # The font's own files (render.sh finds them), so a fontconfig alias that
    # maps the name to another font cannot change the cell grid.
    faces, family = "", FONT
    for var, weight, style in (("FONT_REGULAR", "normal", "normal"), ("FONT_BOLD", "bold", "normal"), ("FONT_ITALIC", "normal", "italic")):
        path = os.environ.get(var)
        if path:
            faces += "@font-face{font-family:'GuideMono';src:url('file://%s');font-weight:%s;font-style:%s}\n" % (path, weight, style)
            family = "GuideMono"
    print("""<!doctype html><meta charset=utf-8><style>
""" + faces + """html,body{margin:0;padding:0;background:#%02x%02x%02x}
#t{position:relative;width:%dpx;height:%dpx;font-family:'%s','Symbols Nerd Font Mono','Noto Sans Mono CJK JP',monospace;font-size:%dpx;line-height:%dpx;overflow:hidden}
.l{position:relative;height:%dpx;white-space:pre}
.r,.c{position:absolute;top:0;height:%dpx;overflow:visible;white-space:pre}
.c{text-align:center}
</style><div id=t>%s</div>""" % (*DEF_BG, W, H, family, FS, CH, CH, CH, "\n".join(body)))
main()
