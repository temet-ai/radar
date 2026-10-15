#!/usr/bin/env python3
"""Ink against Ratatui, one sitting, one binary, one bundle, the offline demo data (T21).

  python3 ink/test/perf/compare.py --radar COPIED_radar --bundle COPIED_radar-ink.js [--node NODE] \
      [--sizes 160x40,80x24] [--runs 2] [--reps 12] [--out DIR] [--json FILE]

Read-only: scratch HOME/XDG folders holding the offline fixtures of tests/ink_pty.rs
(scripts/ink-live/assets/offline), RADAR_MODE=read-only, RADAR_NO_WRITES=1, no token set.
No IPC tap: Ink is started straight from the bundle, so nothing but the renderer is on the path.

Per (renderer, size) it starts radar `--runs` times and, in each, presses every action `--reps` times with the
screen quiet before each press. For a press it records key -> first byte of the PTY output and key -> last
byte (frame complete: the fade-in and any second write included). Actions: the six top-bar screen keys
(J S B I , R), the x menu on Repositories and its Esc, the History overlay (H) and its Esc, single Down/Up/PageDown/PageUp in the Settings nav (the long list), and a held Down (14 presses 33 ms apart) and held PageDown for the tail after the last press.
It also records startup to first frame (the first write holding the top bar's words) and to quiet, and CPU per frame: the
user+system time of radar and its children (node for Ink) over the held-key burst, divided by the writes seen.
Prints median / p95 / max per action, side by side, as markdown, and writes the raw numbers as JSON.
Standard library only; hand-run, not part of `npm test`.
"""
import argparse, fcntl, json, os, pty, re, select, shutil, statistics, struct, sys, tempfile, termios, time

ap = argparse.ArgumentParser()
ap.add_argument("--radar", required=True)
ap.add_argument("--bundle", required=True)
ap.add_argument("--node", default=shutil.which("node") or "node")
ap.add_argument("--sizes", default="160x40,80x24")
ap.add_argument("--renderers", default="ink,ratatui", help="comma list; an Ink variant is label=bundle.js (label=ratatui for the other renderer)")
ap.add_argument("--runs", type=int, default=2)
ap.add_argument("--reps", type=int, default=12)
ap.add_argument("--out", default=None)
ap.add_argument("--json", default=None)
ap.add_argument("--startup", type=int, default=0, help="only startup, this many starts per renderer and size (first visible write, top bar, a key at the first write acting)")
args = ap.parse_args()
args.radar = os.path.abspath(args.radar)
OFFLINE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", ".claude", "skills", "radar-ink-live", "assets", "offline")
out_root = args.out or tempfile.mkdtemp(prefix="radar-cmp-")
os.makedirs(out_root, exist_ok=True)
CLK = os.sysconf("SC_CLK_TCK")


def load1():
    return open("/proc/loadavg").read().split()[0]


def tree_cpu(root):
    """user+system seconds of root and every live descendant."""
    kids, cpu = {}, {}
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            s = open(f"/proc/{p}/stat").read()
            f = s[s.rindex(")") + 2:].split()
            kids.setdefault(int(f[1]), []).append(int(p))
            cpu[int(p)] = (int(f[11]) + int(f[12])) / CLK
        except (OSError, ValueError, IndexError):
            pass
    total, todo = 0.0, [root]
    while todo:
        p = todo.pop()
        total += cpu.get(p, 0.0)
        todo += kids.get(p, [])
    return total


class Session:
    def __init__(self, renderer, size, tag, bundle=None):
        self.cols, self.rows = (int(x) for x in size.split("x"))
        d = os.path.join(out_root, tag)
        home = os.path.join(d, "home")
        for sub in ("home/.config/radar", "home/.local/state/radar", "home/.cache", "home/.local/share"):
            os.makedirs(os.path.join(d, sub), exist_ok=True)
        shutil.copy(os.path.join(OFFLINE, "config.toml"), os.path.join(home, ".config/radar/config.toml"))
        for f in ("gitlab.json", "jira.json"):
            shutil.copy(os.path.join(OFFLINE, f), os.path.join(home, ".local/state/radar", f))
        env = {
            "PATH": os.path.dirname(args.node) + ":" + os.environ.get("PATH", ""),
            "HOME": home, "XDG_CONFIG_HOME": home + "/.config", "XDG_STATE_HOME": home + "/.local/state",
            "XDG_CACHE_HOME": home + "/.cache", "XDG_DATA_HOME": home + "/.local/share",
            "TERM": "xterm-256color", "COLORTERM": "truecolor", "LANG": "C.UTF-8",
            "RADAR_MODE": "read-only", "RADAR_NO_WRITES": "1", "RADAR_RENDERER": renderer,
            "RADAR_INK_BUNDLE": os.path.abspath(bundle or args.bundle),
        }
        self.chunks = []
        self.t_start = time.time()
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", self.rows, self.cols, 0, 0))
            os.chdir(home)
            os.execve(args.radar, [args.radar], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", self.rows, self.cols, 0, 0))
        self.log = os.path.join(home, ".local/state/radar/radar.log")

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.time()))
            if not r:
                return
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                return
            if not data:
                return
            if data == b"\x1b[?25l":  # the host's idle cursor-hide: not a frame
                continue
            self.chunks.append((time.time(), data))
            if b"\x1b[c" in data or b"\x1b[0c" in data:
                os.write(self.fd, b"\x1b[?62;22c")
            if b"\x1b[5n" in data:
                os.write(self.fd, b"\x1b[0n")
            if b"\x1b[6n" in data:
                os.write(self.fd, b"\x1b[1;1R")
            if b"\x1b[16t" in data:
                os.write(self.fd, b"\x1b[6;16;8t")
            if b"\x1b[18t" in data:
                os.write(self.fd, ("\x1b[8;%d;%dt" % (self.rows, self.cols)).encode())

    def quiet(self, gap=0.4, limit=8.0):
        end, last = time.time() + limit, len(self.chunks)
        while time.time() < end:
            self.pump(gap)
            if len(self.chunks) == last:
                return
            last = len(self.chunks)

    def press(self, key):
        """One press with the screen quiet before it: (key -> first byte ms, key -> last byte ms, writes)."""
        self.quiet(gap=0.2)
        n = len(self.chunks)
        t0 = time.time()
        os.write(self.fd, key)
        wait_end = t0 + 5.0
        while len(self.chunks) == n and time.time() < wait_end:
            self.pump(0.02)
        self.quiet(gap=0.4)
        new = self.chunks[n:]
        if not new:
            return None
        return ((new[0][0] - t0) * 1000, (new[-1][0] - t0) * 1000, len(new), sum(len(d) for _, d in new))

    def close(self):
        try:
            os.write(self.fd, b"q")
            time.sleep(0.2)
            os.kill(self.pid, 15)
            os.waitpid(self.pid, 0)
        except OSError:
            pass


ESC_SEQ = re.compile(rb"\x1b(\[[0-9;?<>=]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[@-Z\\-_])")


def visible(data):
    """Whether a write draws words or marks (not only control sequences): five or more printable cells."""
    rest = ESC_SEQ.sub(b"", data)
    return sum(1 for b in rest if b > 0x20 and b != 0x7f) >= 5


def run_startup(renderer, size, tag, bundle=None):
    """One start for --startup: process start -> first visible write (the splash, or the first screen without it),
    -> first write holding the top bar's words, and, in a second start, `,` pressed at the first visible write ->
    the Settings nav's words drawn (the first screen acts on a key at once)."""
    res = {}
    s = Session(renderer, size, tag + "-a", bundle)
    s.pump(0.05)
    s.quiet(gap=1.0, limit=25.0)
    first = next((t for t, d in s.chunks if visible(d)), None)
    bar = next((t for t, d in s.chunks if b"Repositories" in d or b"Settings" in d), None)
    res["first_visible"] = None if first is None else (first - s.t_start) * 1000
    res["first_bar"] = None if bar is None else (bar - s.t_start) * 1000
    s.close()
    s = Session(renderer, size, tag + "-b", bundle)
    end = time.time() + 25.0
    while time.time() < end and not any(visible(d) for _, d in s.chunks):
        s.pump(0.005)
    os.write(s.fd, b",")
    n = len(s.chunks)
    while time.time() < end and not any(b"colours" in d for _, d in s.chunks[n:]):
        s.pump(0.005)
    hit = next((t for t, d in s.chunks[n:] if b"colours" in d), None)
    res["key_acted"] = None if hit is None else (hit - s.t_start) * 1000
    s.close()
    return res


def run_one(renderer, size, tag, reps, bundle=None):
    s = Session(renderer, size, tag, bundle)
    res = {"load_start": load1()}
    s.pump(0.05)
    s.quiet(gap=1.0, limit=25.0)
    # The first frame is the first write that holds the top bar's words: the host draws nothing of its own before it.
    first = next((t for t, d in s.chunks if b"Repositories" in d or b"Settings" in d), None)
    res["startup_first_frame"] = None if first is None else (first - s.t_start) * 1000
    res["startup_quiet"] = (s.chunks[-1][0] - s.t_start) * 1000 if s.chunks else None
    try:  # which parts this radar was built with (the gate's feature mix may lack some)
        res["parts"] = next((l.split("starting: ")[1].strip() for l in open(s.log) if "starting: " in l), None)
    except OSError:
        res["parts"] = None
    if renderer == "ink":  # prove Ink drew it (radar.log says "Ink renderer started")
        try:
            res["ink_started"] = "Ink renderer started" in open(s.log).read()
        except OSError:
            res["ink_started"] = False
    acts = {}

    def rec(name, key, n):
        for _ in range(n):
            r = s.press(key)
            if r:
                acts.setdefault(name, []).append(r)

    # Screen switches, in a loop, each key from the previous screen.
    for _ in range(reps):
        for name, key in (("screen J (Jira)", b"J"), ("screen S (Slack)", b"S"), ("screen B (Branches)", b"B"),
                          ("screen I (Work)", b"I"), ("screen , (Settings)", b","), ("screen R (Repositories)", b"R")):
            r = s.press(key)
            if r:
                acts.setdefault(name, []).append(r)
    # Menu on Repositories.
    for _ in range(reps):
        for name, key in (("menu x open", b"x"), ("menu Esc close", b"\x1b")):
            r = s.press(key)
            if r:
                acts.setdefault(name, []).append(r)
    # History overlay.
    for _ in range(reps):
        for name, key in (("History H open", b"H"), ("History Esc close", b"\x1b")):
            r = s.press(key)
            if r:
                acts.setdefault(name, []).append(r)
    # The long list: Settings' nav tree (about 20 rows). `1` focuses the nav pane.
    s.press(b",")
    s.press(b"1")
    for _ in range(reps):
        for name, key in (("list Down (one key)", b"\x1b[B"), ("list PageDown (one key)", b"\x1b[6~"),
                          ("list Up (one key)", b"\x1b[A"), ("list PageUp (one key)", b"\x1b[5~")):
            r = s.press(key)
            if r:
                acts.setdefault(name, []).append(r)
    s.press(b"\x1b[5~")
    # Held Down x14 then held Up x14 (33 ms apart, a key repeat): the tail is last press -> last byte.
    # CPU per frame over the Down burst.
    held, cpu_frames = [], []
    for _ in range(max(3, reps // 3)):
        for key in (b"\x1b[B", b"\x1b[A"):
            s.quiet(gap=0.4)
            n = len(s.chunks)
            c0 = tree_cpu(s.pid)
            w0 = time.time()
            for i in range(14):
                os.write(s.fd, key)
                t_last = time.time()
                s.pump(0.033)
            s.quiet(gap=0.4)
            c1 = tree_cpu(s.pid)
            new = s.chunks[n:]
            if new:
                held.append(((new[-1][0] - t_last) * 1000, len(new), sum(len(d) for _, d in new), (new[0][0] - w0) * 1000))
                cpu_frames.append(((c1 - c0) * 1000 / len(new), (c1 - c0) * 1000))
    # Held PageDown x6 then PageUp x6.
    held_pd = []
    for _ in range(max(3, reps // 3)):
        for key in (b"\x1b[6~", b"\x1b[5~"):
            s.quiet(gap=0.4)
            n = len(s.chunks)
            for i in range(6):
                os.write(s.fd, key)
                t_last = time.time()
                s.pump(0.033)
            s.quiet(gap=0.4)
            new = s.chunks[n:]
            if new:
                held_pd.append(((new[-1][0] - t_last) * 1000, len(new), sum(len(d) for _, d in new), 0))
    # Idle CPU, 3 s.
    s.quiet(gap=0.4)
    i0, w0 = tree_cpu(s.pid), time.time()
    s.pump(3.0)
    res["idle_cpu_pct"] = (tree_cpu(s.pid) - i0) / (time.time() - w0) * 100
    res["load_end"] = load1()
    res["acts"] = acts
    res["held_j"] = held
    res["held_pd"] = held_pd
    res["cpu_per_frame"] = cpu_frames
    s.close()
    return res


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return None
    k = (len(xs) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def stats(xs):
    return None if not xs else (statistics.median(xs), pct(xs, 95), max(xs))


def f(t):
    return "-" if t is None else "%.0f / %.0f / %.0f" % t


sizes = args.sizes.split(",")
renderers = args.renderers.split(",")
all_res = {}
if args.startup:
    for size in sizes:
        for run in range(args.startup):
            for r in renderers:
                kind, bundle = ("ratatui", None) if r == "ratatui" else ("ink", r.split("=", 1)[1] if "=" in r else None)
                all_res.setdefault((size, r), []).append(run_startup(kind, size, f"{r.split('=')[0]}-{size}-s{run}", bundle))
    if args.json:
        json.dump({f"{k[0]}|{k[1]}": v for k, v in all_res.items()}, open(args.json, "w"))
    for size in sizes:
        print(f"\n### startup {size}  (median / p95 / max, ms; {args.startup} starts)\n")
        labels = [r.split("=")[0] for r in renderers]
        print("| measure | " + " | ".join(labels) + " |")
        print("|---|" + "---|" * len(renderers))
        for label, key in (("process start -> first visible write", "first_visible"),
                           ("process start -> top bar drawn (no key)", "first_bar"),
                           ("process start -> `,` at first write has acted", "key_acted")):
            print("| " + label + " | " + " | ".join(f(stats([x[key] for x in all_res[(size, r)] if x[key] is not None])) for r in renderers) + " |")
    print(f"\nscratch: {out_root}")
    sys.exit(0)
print(f"load average at start: {open('/proc/loadavg').read().strip()}", flush=True)
for size in sizes:
    for run in range(args.runs):
        for r in renderers:  # alternate within a size: the machine's state is shared
            print(f"... {r} {size} run {run + 1}/{args.runs} (load {load1()})", file=sys.stderr, flush=True)
            kind, bundle = ("ratatui", None) if r == "ratatui" else ("ink", r.split("=", 1)[1] if "=" in r else None)
            if "=" in r and r.split("=", 1)[1] == "ratatui":
                kind, bundle = "ratatui", None
            all_res.setdefault((size, r), []).append(run_one(kind, size, f"{r.split('=')[0]}-{size}-{run}", args.reps, bundle))
if args.json:
    json.dump({f"{k[0]}|{k[1]}": v for k, v in all_res.items()}, open(args.json, "w"))

for size in sizes:
    print(f"\n### {size}  (median / p95 / max, ms; {args.runs} runs x {args.reps} presses per action)\n")
    base = {r: all_res[(size, r)] for r in renderers}
    for r in renderers:
        loads = [x["load_start"] for x in base[r]] + [x["load_end"] for x in base[r]]
        extra = "" if r == "ratatui" or r.endswith("=ratatui") else "; Ink drew: " + str([x.get("ink_started") for x in base[r]])
        print(f"- {r}: radar {base[r][0].get('parts')}")
        print(f"- {r}: load averages seen {', '.join(loads)}{extra}")
    labels = [r.split("=")[0] for r in renderers]
    print("\n| action | " + " | ".join(f"{r} first byte" for r in labels) + " | " + " | ".join(f"{r} frame complete" for r in labels) + " |")
    print("|---|" + "---|" * (2 * len(renderers)))
    names = []
    for r in renderers:
        for x in base[r]:
            for n in x["acts"]:
                if n not in names:
                    names.append(n)
    for n in names:
        row = [n]
        for idx in (0, 1):
            for r in renderers:
                v = [a[idx] for x in base[r] for a in x["acts"].get(n, [])]
                row.append(f(stats(v)))
        print("| " + " | ".join(row) + " |")
    print("\n| measure | " + " | ".join(labels) + " |")
    print("|---|" + "---|" * len(renderers))
    for label, key in (("startup: process start -> first frame (ms)", "startup_first_frame"), ("startup: process start -> quiet (ms)", "startup_quiet")):
        print("| " + label + " | " + " | ".join(f(stats([x[key] for x in base[r] if x[key] is not None])) for r in renderers) + " |")
    print("| held Down x14 @33 ms: last press -> last byte (ms) | " + " | ".join(f(stats([h[0] for x in base[r] for h in x["held_j"]])) for r in renderers) + " |")
    print("| held Down x14: terminal writes (count, median) | " + " | ".join("%.0f" % statistics.median([h[1] for x in base[r] for h in x["held_j"]] or [0]) for r in renderers) + " |")
    print("| held Down x14: bytes written (median) | " + " | ".join("%.0f" % statistics.median([h[2] for x in base[r] for h in x["held_j"]] or [0]) for r in renderers) + " |")
    print("| held PageDown x6 @33 ms: last press -> last byte (ms) | " + " | ".join(f(stats([h[0] for x in base[r] for h in x["held_pd"]])) for r in renderers) + " |")
    print("| CPU per terminal write over the held-Down burst, radar+node (ms) | " + " | ".join(f(stats([c[0] for x in base[r] for c in x["cpu_per_frame"]])) for r in renderers) + " |")
    print("| CPU total over the held-Down burst (ms) | " + " | ".join(f(stats([c[1] for x in base[r] for c in x["cpu_per_frame"]])) for r in renderers) + " |")
    print("| idle CPU, 3 s (% of one core) | " + " | ".join("%.1f" % statistics.median([x["idle_cpu_pct"] for x in base[r]]) for r in renderers) + " |")
print(f"\nscratch: {out_root}")
