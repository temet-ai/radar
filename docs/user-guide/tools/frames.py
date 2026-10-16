"""frames.py OUT_DIR FPS CMD...: grab the screen for a clip.

Runs CMD (one `ink-live colour NAME`) FPS times a second and writes each screen that
differs from the one before as OUT_DIR/NNNN.ans, with the time it appeared in
OUT_DIR/times.txt (seconds from the start, one line per frame). Stops when OUT_DIR/stop
exists, and writes the end time as the last line of times.txt.
"""

import os
import subprocess
import sys
import time


def main() -> None:
    out, fps, cmd = sys.argv[1], float(sys.argv[2]), sys.argv[3:]
    os.makedirs(out, exist_ok=True)
    stop = os.path.join(out, "stop")
    start = time.monotonic()
    last = None
    times = []
    while not os.path.exists(stop):
        tick = time.monotonic()
        screen = subprocess.run(cmd, capture_output=True, check=False).stdout
        if screen and screen != last:
            with open(os.path.join(out, f"{len(times):04d}.ans"), "wb") as f:
                f.write(screen)
            times.append(tick - start)
            last = screen
        time.sleep(max(0.0, 1.0 / fps - (time.monotonic() - tick)))
    times.append(time.monotonic() - start)
    with open(os.path.join(out, "times.txt"), "w") as f:
        f.write("".join(f"{t:.3f}\n" for t in times))


if __name__ == "__main__":
    main()
