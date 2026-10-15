# usage: drive.py <node> <script.mjs>; types ESC, j, down, ESC, x into a pty 200 ms apart and prints what the child wrote.
import os, pty, select, sys, time

pid, fd = pty.fork()
if pid == 0:
    os.execvp(sys.argv[1], [sys.argv[1], sys.argv[2]])
buf = b""
def drain(t):
    global buf
    end = time.time() + t
    while time.time() < end:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                d = os.read(fd, 4096)
            except OSError:
                return
            if not d:
                return
            buf += d
while b"ready" not in buf:
    drain(0.1)
for k in [b"\x1b", b"j", b"\x1b[B", b"\x1b", b"x"]:
    os.write(fd, k)
    drain(0.2)
drain(0.3)
sys.stdout.write(buf.decode(errors="replace"))
os.kill(pid, 9)
