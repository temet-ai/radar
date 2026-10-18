#!/usr/bin/env python3
"""Private single-Cargo supervisor for scripts/scenarios; Python 3.9+ POSIX."""

import os
import signal
import struct
import subprocess
import sys

REPORT = struct.Struct("!4sBi")
REPORT_MAGIC = b"RSC1"
ABANDONED_RETURN_CODE = 255  # Destructor bookkeeping, never actual exit evidence.


def diagnostic(message):
    print(f"Scenario supervisor: {message[:512]}", file=sys.stderr, flush=True)


def report_and_wait(descriptor, kind, value):
    record = REPORT.pack(REPORT_MAGIC, kind, value)
    try:
        if os.write(descriptor, record) != len(record):
            diagnostic("short status write")
            return 1
    except OSError as error:
        diagnostic(f"status write failed: {error}")
        return 1
    finally:
        try:
            os.close(descriptor)
        except OSError as error:
            diagnostic(f"status close failed: {error}")
    # Cargo's status has been reported, but this PID remains the parent's owned
    # group identity until its TERM/KILL cleanup. Do not exit on normal success.
    while True:
        signal.pause()


def main(arguments=None):
    args = sys.argv[1:] if arguments is None else arguments
    if len(args) < 3 or not args[0].isascii() or not args[0].isdecimal() or args[1] != "--":
        diagnostic("invalid private invocation")
        return 1
    descriptor = int(args[0])
    if descriptor < 3:
        diagnostic("invalid private status descriptor")
        return 1
    try:
        os.set_inheritable(descriptor, False)
        if os.getpid() != os.getpgrp() or os.getpid() != os.getsid(0):
            raise ValueError("supervisor must lead its own session and process group")
        if signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL:
            raise ValueError("supervisor requires default SIGCHLD handling")
    except (OSError, ValueError) as error:
        diagnostic(str(error))
        return report_and_wait(descriptor, 3, 1)
    # No shell, replacement environment, new session, private inherited FD, or
    # signal-disposition changes. Cargo supplies its own test runtime environment.
    try:
        cargo = subprocess.Popen(args[2:], close_fds=True, pass_fds=())
    except OSError as error:
        diagnostic(f"could not launch Cargo: {error}")
        number = error.errno if error.errno and 1 <= error.errno <= 4095 else 1
        return report_and_wait(descriptor, 2, number)
    try:
        pid, status = os.waitpid(cargo.pid, 0)
        if pid != cargo.pid:
            raise ChildProcessError("unexpected Cargo child from waitpid")
        cargo.returncode = os.waitstatus_to_exitcode(status)
    except (OSError, ValueError) as error:
        cargo.returncode = ABANDONED_RETURN_CODE
        diagnostic(f"could not establish Cargo exit status: {error}")
        return report_and_wait(descriptor, 3, 1)
    return report_and_wait(descriptor, 1, cargo.returncode)


if __name__ == "__main__":
    sys.exit(main())
