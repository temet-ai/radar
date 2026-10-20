#!/usr/bin/env python3
"""Key -> first byte of the new frame, in the real radar binary with the Ink renderer, under a PTY (T22).

Read-only: scratch HOME/XDG folders, RADAR_MODE=read-only, RADAR_NO_WRITES=1, the demo data (no config, no provider).
Nothing under ~/.config/radar is touched and nothing is installed.

  python3 ink/test/perf/pty_latency.py --radar PATH_TO_COPIED_radar --bundle PATH_TO_COPIED_radar-ink.js \
      [--node NODE] [--size 160x40] [--node-env production|development|unset] [--out DIR]

--radar is a copy of a default-features `cargo build --release --bin radar` (never target/fleet: the gate's feature mix may lack parts; check radar.log says "tui with gitlab, github, jira, slack, git, workspace, workbench"), --bundle a copy of ink/dist/radar-ink.js
(`npm run bundle` in ink/). The bundle is started through ipc-tap.mjs, which logs every IPC message
(direction, bytes, type) and every terminal write with a wall-clock stamp to <out>/ipc.log; this script
stamps every key it writes, so one keypress reads as: key written -> intent sent by Ink -> snapshot from
the host -> first terminal write -> frame quiet. Prints one table row per key.

Hand-run measurement tool (python3 standard library only; not part of `npm test`). Switches by environment:
PTY_KEYS="Down,Enter,x,Esc,resize" (the keys to press; `resize` changes the width by 10 columns),
PTY_GAP seconds of quiet before each key, PTY_DUMP=1 prints the last screen, PTY_NO_TAP=1 runs the bundle without
the tap. The binary must match the bundle (same commit of the tree).
"""
import argparse, fcntl, os, pty, select, shutil, struct, sys, tempfile, termios, time

ap = argparse.ArgumentParser()
ap.add_argument("--radar", required=True)
ap.add_argument("--bundle", required=True)
ap.add_argument("--node", default=shutil.which("node") or "node")
ap.add_argument("--size", default="160x40")
ap.add_argument("--node-env", default="unset", choices=["production", "development", "unset"])
ap.add_argument("--out", default=None)
ap.add_argument("--downs", type=int, default=3)
args = ap.parse_args()
cols, rows = (int(x) for x in args.size.split("x"))
out = args.out or tempfile.mkdtemp(prefix="radar-perf-")
os.makedirs(out, exist_ok=True)
home = os.path.join(out, "home")
for d in ("", ".config", ".local/state", ".cache", ".local/share"):
    os.makedirs(os.path.join(home, d), exist_ok=True)
# PTY_OFFLINE=1: start on ink-live's offline data (GitLab acme/team-a, Jira ABC-12) instead of a first run, so the
# keys move through real screens (J, R, B, W switch screens there).
if os.environ.get("PTY_OFFLINE"):
    offline = os.path.join(os.path.dirname(os.path.abspath(__file__)), "../../../scripts/ink-live/assets/offline")
    os.makedirs(os.path.join(home, ".config/radar"), exist_ok=True)
    os.makedirs(os.path.join(home, ".local/state/radar"), exist_ok=True)
    shutil.copy(os.path.join(offline, "config.toml"), os.path.join(home, ".config/radar/config.toml"))
    for f in ("gitlab.json", "jira.json"):
        shutil.copy(os.path.join(offline, f), os.path.join(home, ".local/state/radar", f))
    # PTY_CONFIG_EXTRA: TOML appended to that config (e.g. `[theme]\nmotion = "off"` to time a frame without motion).
    if os.environ.get("PTY_CONFIG_EXTRA"):
        with open(os.path.join(home, ".config/radar/config.toml"), "a") as c:
            c.write("\n" + os.environ["PTY_CONFIG_EXTRA"].replace("\\n", "\n") + "\n")
log = os.path.join(out, "ipc.log")
open(log, "w").close()
tap = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ipc-tap.mjs")

env = {
    "PATH": os.path.dirname(args.node) + ":" + os.environ.get("PATH", ""),
    "HOME": home,
    "XDG_CONFIG_HOME": home + "/.config",
    "XDG_STATE_HOME": home + "/.local/state",
    "XDG_CACHE_HOME": home + "/.cache",
    "XDG_DATA_HOME": home + "/.local/share",
    "TERM": "xterm-256color",
    "COLORTERM": "truecolor",
    "LANG": "C.UTF-8",
    "RADAR_MODE": "read-only",
    "RADAR_NO_WRITES": "1",
    "RADAR_INK_BUNDLE": os.path.abspath(args.bundle) if os.environ.get("PTY_NO_TAP") else tap,
    "RADAR_PERF_BUNDLE": os.path.abspath(args.bundle),
    "RADAR_PERF_LOG": log,
}
if os.environ.get("PTY_NODE_OPTIONS"):  # e.g. "--cpu-prof --cpu-prof-dir=/path" to profile the renderer
    env["NODE_OPTIONS"] = os.environ["PTY_NODE_OPTIONS"]
if args.node_env != "unset":
    env["NODE_ENV"] = args.node_env

pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))  # the size before the program starts
    os.execve(args.radar, [args.radar], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

chunks = []  # (time, bytes)


def pump(seconds):
    """Read the PTY for up to `seconds`, answering the terminal queries Ink and the host send."""
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], max(0, end - time.time()))
        if not r:
            return
        try:
            data = os.read(fd, 65536)
        except OSError:
            return
        if not data:
            return
        if data == b"\x1b[?25l":  # the host's idle cursor-hide, every 200 ms: not a frame, and it would keep `quiet` waiting
            continue
        chunks.append((time.time(), data))
        if os.environ.get("PTY_NO_REPLY"):
            continue
        # Queries, answered as an xterm-like terminal with no graphics (the host reads the replies from stdin and
        # would keep waiting for - and swallow - the next keys if they did not come).
        if b"\x1b[c" in data or b"\x1b[0c" in data:
            os.write(fd, b"\x1b[?62;22c")
        if b"\x1b[5n" in data:
            os.write(fd, b"\x1b[0n")
        if b"\x1b[6n" in data:
            os.write(fd, b"\x1b[1;1R")
        if b"\x1b[16t" in data:
            os.write(fd, b"\x1b[6;16;8t")
        if b"\x1b[18t" in data:
            os.write(fd, ("\x1b[8;%d;%dt" % (rows, cols)).encode())


def quiet(gap=0.4, limit=8.0):
    """Pump until the terminal has been silent for `gap` seconds."""
    end = time.time() + limit
    last = len(chunks)
    while time.time() < end:
        pump(gap)
        if len(chunks) == last:
            return
        last = len(chunks)


pump(0.1)
t_start = time.time()
quiet(gap=1.0, limit=20.0)
first_paint = next((t for t, d in chunks if len(d) > 500), None)
print(f"startup: process start -> first frame {None if first_paint is None else round((first_paint - t_start) * 1000)} ms;"
      f" size {cols}x{rows}; NODE_ENV={args.node_env}; radar={os.path.basename(args.radar)}")

DOWN, ENTER, ESC = b"\x1b[B", b"\r", b"\x1b"
steps = [("Down", DOWN)] * args.downs + [("Enter", ENTER), ("x", b"x"), ("Esc", ESC), ("Esc", ESC), ("Down", DOWN)]
KEYS = {"Down": DOWN, "Enter": ENTER, "Esc": ESC, "Up": b"\x1b[A", "Right": b"\x1b[C", "Tab": b"\t"}
if os.environ.get("PTY_KEYS"):  # e.g. PTY_KEYS="Down,Right,x,Esc"
    steps = [(k, KEYS.get(k, k.encode())) for k in os.environ["PTY_KEYS"].split(",")]
rows_out = []
for name, key in steps:
    quiet(gap=float(os.environ.get("PTY_GAP", "0.3")))
    n = len(chunks)
    t0 = time.time()
    if name == "resize":  # the terminal changes size (SIGWINCH), no key
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols - 10, 0, 0))
    else:
        os.write(fd, key)
    # The first byte can be late (that is what is measured): wait for it, then for the frame to go quiet.
    wait_end = time.time() + 5.0
    while len(chunks) == n and time.time() < wait_end:
        pump(0.05)
    quiet(gap=0.4)
    new = chunks[n:]
    first = (new[0][0] - t0) * 1000 if new else None
    last = (new[-1][0] - t0) * 1000 if new else None
    size = sum(len(d) for _, d in new)
    rows_out.append((name, t0, first, last, size, len(new)))

try:
    os.write(fd, b"q")
    time.sleep(0.3)
    os.kill(pid, 15)
except OSError:
    pass

# Merge with the IPC log: per key, what crossed the socket between the key and the quiet period.
events = []
for l in open(log):
    p = l.split(None, 3)
    if len(p) == 4:
        events.append((float(p[0]) / 1000.0, p[1], int(p[2]), p[3].strip()))


def ms(t, t0):
    return "-" if t is None else "%.0f" % ((t - t0) * 1000)


print("\nkey | key->first byte ms | key->last byte ms | frame bytes | writes | key->intent ms | intent->snapshot ms | snapshot->tty ms | IPC out bytes | IPC in bytes | IPC in")
for name, t0, first, last, size, nwrites in rows_out:
    window = [e for e in events if t0 <= e[0] <= t0 + 0.3 + (last or 0) / 1000]
    intent = next((e for e in window if e[1] == "out" and e[3].startswith("intent")), None)
    snap = next((e for e in window if e[1] == "in" and e[3] == "snapshot"), None)
    tty = next((e for e in window if e[1] == "tty" and (snap is None or e[0] >= snap[0])), None)
    out_b = sum(e[2] for e in window if e[1] == "out")
    in_b = sum(e[2] for e in window if e[1] == "in")
    in_kinds = ",".join(f"{e[3]}={e[2]}" for e in window if e[1] == "in")
    print(" | ".join([
        name,
        "-" if first is None else "%.0f" % first,
        "-" if last is None else "%.0f" % last,
        str(size), str(nwrites),
        ms(intent and intent[0], t0),
        "-" if not (intent and snap) else "%.0f" % ((snap[0] - intent[0]) * 1000),
        "-" if not (snap and tty) else "%.0f" % ((tty[0] - snap[0]) * 1000),
        str(out_b), str(in_b), in_kinds,
    ]))
if os.environ.get("PTY_ESC"):
    import re
    print(sorted({m.encode() for _, d in chunks for m in re.findall(r"\x1b\[[?>=0-9;]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*", d.decode("utf8", "replace")) if not re.match(r"\x1b\[[0-9;]*[mHJKABCD]$", m)}))
if os.environ.get("PTY_DUMP"):
    import re
    print(re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", b"".join(d for _, d in chunks[-30:]).decode("utf8", "replace"))[-3000:])
print(f"\nlogs and scratch homes: {out}")
