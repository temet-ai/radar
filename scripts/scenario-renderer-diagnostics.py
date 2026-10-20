#!/usr/bin/env python3
"""Bounded diagnostics for the single committed GitHub 120x40 renderer checkpoint.

Preflight checks a small recipe-specific shape, not the full wire protocol. Only
producer freshness followed by a real Ink replay can qualify normalized text.
No styles, display-cell geometry, pixels or live terminal behavior are checked.
"""

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = Path("ink/test/fixtures/scenarios/github_refresh_120x40")
MARKER = "Patrol the Brass Bridge"
LOG_BYTES = 16 * 1024
INPUT_BYTES = 512 * 1024
# Match the existing Ink test's plain() and ECMAScript trimEnd(), not terminal cells.
CSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
TRAILING = re.compile(r"[\t\n\v\f\r \u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff]+$")
STAGES = ("preflight", "dependencies", "producer-freshness", "ink", "text-parity")


class Failure(Exception):
    def __init__(self, stage, message, code=1):
        super().__init__(message)
        self.stage, self.code = stage, code


def normalize(text):
    rows = [TRAILING.sub("", row) for row in CSI.sub("", text).split("\n")]
    while rows and rows[-1] == "":
        rows.pop()
    return "\n".join(rows)


def text_diff(expected, actual):
    """First difference and at most five clipped rows; 1-based Unicode codepoints."""
    left, right = expected.split("\n"), actual.split("\n")
    for row in range(max(len(left), len(right))):
        a = left[row] if row < len(left) else None
        b = right[row] if row < len(right) else None
        if a == b:
            continue
        a_text, b_text = a or "", b or ""
        col = next((i for i, pair in enumerate(zip(a_text, b_text)) if pair[0] != pair[1]),
                   min(len(a_text), len(b_text)))
        start = max(0, col - 40)
        context = []
        for n in range(max(0, row - 2), min(max(len(left), len(right)), row + 3)):
            context.append({"row": n + 1, "from_codepoint_column": start + 1,
                            "expected": left[n][start:start + 120] if n < len(left) else None,
                            "actual": right[n][start:start + 120] if n < len(right) else None})
        return {"row": row + 1, "codepoint_column": col + 1,
                "coordinate_unit": "1-based Unicode codepoints, not display cells or pixels",
                "context": context, "context_limit": "5 rows, 120 codepoints per side per row"}
    return None


def fresh_output(requested, root):
    if requested is None:
        # Deliberately independent of the checkout and of ambient TMPDIR settings.
        return Path(tempfile.mkdtemp(prefix="radar-renderer-", dir="/tmp")).resolve()
    raw = Path(requested)
    if ".." in raw.parts:
        raise Failure("artifacts", "output path must not contain '..'")
    path = Path(os.path.abspath(raw))
    if path == root or root in path.parents:
        raise Failure("artifacts", "output must be outside the repository and fixture inputs")
    for ancestor in [path, *path.parents]:
        if ancestor.is_symlink():
            raise Failure("artifacts", f"refusing symlink output path: {ancestor}")
    try:
        path.mkdir(mode=0o700)  # No parents=True; never alter caller's existing directories.
    except OSError as error:
        raise Failure("artifacts", f"use a new directory under an existing parent: {error}") from error
    return path


def write_new(path, text):
    # Exclusive creation also refuses a final-component symlink. All artifacts live
    # in our private, newly created directory; nothing overwrites fixture inputs.
    with path.open("x", encoding="utf-8") as output:
        output.write(text)


def read_bounded(path, stage):
    try:
        if not stat.S_ISREG(path.lstat().st_mode):
            raise Failure(stage, f"{path.name} must be a regular file, not a symlink or special file")
        with path.open("rb") as source:
            data = source.read(INPUT_BYTES + 1)
        if len(data) > INPUT_BYTES:
            raise Failure(stage, f"{path.name} exceeds the {INPUT_BYTES}-byte input limit")
        return data.decode("utf-8")
    except (OSError, UnicodeError) as error:
        raise Failure(stage, f"cannot read {path.name}: {error}") from error


def reject_constant(value):
    raise ValueError(f"non-JSON numeric constant {value}")


def preflight(root):
    source = root / FIXTURE.with_suffix(".json")
    expected_path = root / FIXTURE.with_suffix(".txt")
    raw = read_bounded(source, "preflight")
    expected_raw = read_bounded(expected_path, "preflight")
    try:
        view = json.loads(raw, parse_constant=reject_constant)
    except (ValueError, RecursionError) as error:
        raise Failure("preflight", f"malformed checkpoint JSON: {error}") from error
    if not isinstance(view, dict):
        raise Failure("preflight", "checkpoint must be a JSON object")
    if any(type(view.get(k)) is not int for k in ("width", "height")) or \
            [view["width"], view["height"]] != [120, 40]:
        raise Failure("preflight", "checkpoint dimensions must be integer width=120, height=40")
    chrome, screen = view.get("chrome"), view.get("screen")
    if not isinstance(chrome, dict) or not isinstance(screen, dict) or \
            view.get("focus") not in ("nav", "middle", "detail") or \
            "section" not in view or view["section"] is not None or \
            not isinstance(screen.get("panes"), list) or not screen["panes"]:
        raise Failure("preflight", "basic checkpoint shape requires chrome, screen panes, focus and null section")
    if any(type(chrome.get(key)) is not int for key in ("width", "height", "version")) or \
            [chrome.get("width"), chrome.get("height")] != [120, 40] or \
            chrome.get("version") != 1 or type(screen.get("version")) is not int or screen.get("version") != 1:
        raise Failure("preflight", "chrome dimensions/version or screen version differ from this fixed recipe")
    if screen.get("scope") != "github" or screen.get("page"):
        raise Failure("preflight", "semantic checkpoint must be the GitHub overview (scope=github, no page)")
    expected = normalize(expected_raw)
    if len(expected_raw.splitlines()) != 40:
        raise Failure("preflight", "text fixture must contain 40 rows; display-cell widths are not checked")
    if MARKER not in json.dumps(screen, ensure_ascii=False) or MARKER not in expected:
        raise Failure("preflight", f"both fixtures must contain semantic marker {MARKER!r}")
    identities = [{"path": str(path.relative_to(root)), "sha256": hashlib.sha256(text.encode()).hexdigest()}
                  for path, text in ((source, raw), (expected_path, expected_raw))]
    return expected, identities


def verify_fixture_identity(root, identities, stage):
    for identity in identities:
        current = read_bounded(root / identity["path"], stage)
        if hashlib.sha256(current.encode()).hexdigest() != identity["sha256"]:
            raise Failure(stage, f"fixture changed since preflight: {identity['path']}; rerun with stable inputs")


@dataclass
class CommandResult:
    returncode: int
    output: bytes
    timed_out: bool = False


def run_process(argv, *, cwd, env, timeout):
    """Drain output while retaining only a bounded tail; kill/reap the POSIX group."""
    process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, start_new_session=True)
    tail = bytearray()
    deadline = time.monotonic() + timeout
    timed_out = False
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if time.monotonic() >= deadline:
                    timed_out = True
                    os.killpg(process.pid, signal.SIGKILL)
                    break
                for key, _ in selector.select(min(0.1, max(0, deadline - time.monotonic()))):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if chunk:
                        tail.extend(chunk)
                        del tail[:-LOG_BYTES]
                    else:
                        selector.unregister(key.fileobj)
            # stdout can close before the command exits; bound that wait too.
            try:
                process.wait(timeout=max(0.001, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        raise
    finally:
        process.stdout.close()
    return CommandResult(process.returncode, bytes(tail), timed_out)


def check_dependencies(root, env):
    missing = [name for name in ("cargo", "node", "npm") if shutil.which(name, path=env.get("PATH")) is None]
    if not (root / "ink/node_modules/.bin/vitest").is_file():
        missing.append("ink/node_modules/.bin/vitest (install the locked Ink dependencies separately)")
    if missing:
        raise Failure("dependencies", "missing: " + ", ".join(missing))


def run_checks(root, output, *, preflight_only=False, timeout=300, runner=run_process,
               dependency_check=check_dependencies, environ=None):
    report = {"recipe": "github-denied-detail", "dimensions": {"width": 120, "height": 40},
              "source": "tests/provider_scenarios.rs::github::held_refresh_then_denied_detail_preserves_rows_and_persists_failure",
              "status": "failed", "qualification": "NOT QUALIFIED: all required stages must pass", "scope": "normalized text only; styles, display-cell geometry, pixels and live terminal behavior not checked",
              "normalization": "strip CSI matching ESC[0-9;?]*[A-Za-z], trim ECMAScript trailing whitespace per row, drop final blank rows",
              "rerun": "scripts/check-scenario-renderers" + (" --preflight-only" if preflight_only else ""),
              "stages": {stage: {"status": "not-run"} for stage in STAGES}}
    env = dict(os.environ if environ is None else environ)
    env.pop("RADAR_SCENARIO_FIXTURES_DIR", None)
    env.pop("RADAR_SCENARIO_ACTUAL_OUTPUT", None)
    env["INSTA_UPDATE"] = "no"
    code = 1
    try:
        expected, report["fixtures"] = preflight(root)
        report["producer_source_sha256"] = hashlib.sha256(
            read_bounded(root / "tests/provider_scenarios.rs", "preflight").encode()).hexdigest()
        write_new(output / "expected.normalized.txt", expected + "\n")
        report["stages"]["preflight"] = {"status": "passed", "evidence": "basic recipe shape and marker only; no full protocol validation"}
        print("[preflight] basic recipe checks passed (not full protocol validation)", flush=True)
        if preflight_only:
            report["status"] = "preflight-only"
            report["qualification"] = "NOT QUALIFIED: producer freshness, Ink replay and text parity not run"
            code = 0
        else:
            dependency_check(root, env)
            report["stages"]["dependencies"] = {"status": "passed"}
            for stage, argv, cwd in (
                ("producer-freshness", [sys.executable, str(root / "scripts/scenarios"), "github-denied-detail"], root),
                ("ink", ["npm", "test", "--", "--maxWorkers=1", "test/scenarios/checkpoints.test.tsx"], root / "ink"),
            ):
                command_env = dict(env)
                if stage == "ink":
                    command_env["RADAR_SCENARIO_ACTUAL_OUTPUT"] = str(output / "actual.ansi.txt")
                print(f"[{stage}] running fixed check (timeout {timeout:g}s)", flush=True)
                try:
                    result = runner(argv, cwd=cwd, env=command_env, timeout=timeout)
                except (OSError, subprocess.SubprocessError) as error:
                    raise Failure(stage, f"could not run fixed command: {error}") from error
                tail = result.output[-LOG_BYTES:].decode("utf-8", errors="replace")
                # Replacement characters can expand invalid UTF-8; bound the saved
                # UTF-8 log too, not only the subprocess byte buffer.
                tail = tail.encode("utf-8")[-LOG_BYTES:].decode("utf-8", errors="ignore")
                write_new(output / f"{stage}.log", tail)
                report["stages"][stage] = {"status": "failed" if result.returncode or result.timed_out else "passed",
                                           "exit_code": result.returncode, "timed_out": result.timed_out,
                                           "command": argv, "log": f"{stage}.log", "log_tail_limit_bytes": LOG_BYTES}
                if stage == "ink":
                    actual_path = output / "actual.ansi.txt"
                    if actual_path.is_symlink():
                        raise Failure("ink", "refusing symlink actual frame artifact")
                    if actual_path.exists():
                        actual = normalize(read_bounded(actual_path, "ink"))
                        write_new(output / "actual.normalized.txt", actual + "\n")
                        report["first_difference"] = text_diff(expected, actual)
                        report["stages"]["text-parity"] = {
                            "status": "failed" if report["first_difference"] else "passed",
                            "evidence": "comparison of captured normalized strings only; does not qualify replay when another stage fails",
                        }
                    elif not result.returncode and not result.timed_out:
                        raise Failure("ink", "command exited successfully but no actual frame was captured; replay is unverified")
                # This gate runs before Ink and after replay, so a changing pair
                # cannot be attributed to the original expected/hash artifacts.
                verify_fixture_identity(root, report["fixtures"], stage)
                if result.timed_out:
                    raise Failure(stage, f"timed out after {timeout:g}s; process group killed; inspect {stage}.log", 124)
                if result.returncode:
                    raise Failure(stage, f"fixed command exited {result.returncode}; inspect {stage}.log", result.returncode)
                if stage == "ink" and report.get("first_difference"):
                    raise Failure("text-parity", "normalized Ink text and the committed text frame differ")
            report["status"] = "passed"
            report["qualification"] = "fresh producer checkpoint and real Ink replay agree on normalized text only"
            code = 0
    except Failure as error:
        report["failed_stage"], report["error"] = error.stage, str(error)
        report["stages"].setdefault(error.stage, {})["status"] = "failed"
        code = error.code if error.code > 0 else 1
    except OSError as error:
        report["failed_stage"], report["error"] = "artifacts", f"cannot write diagnostics: {error}"
        report["stages"]["artifacts"] = {"status": "failed"}
    report["exit_code"] = code
    write_new(output / "report.json", json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(f"[{report.get('failed_stage', report['status'])}] {report.get('error', report.get('qualification'))}")
    if report.get("first_difference"):
        diff = report["first_difference"]
        print(f"First difference: row {diff['row']}, Unicode codepoint column {diff['codepoint_column']} (1-based; not display cells or pixels).")
        for row in diff["context"]:
            print(f"  row {row['row']} expected={row['expected']!r} actual={row['actual']!r}")
    print(f"Artifacts: {output}\nRerun: {report['rerun']}")
    return code


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight-only", action="store_true", help="cheap fixture checks only; does not qualify parity")
    parser.add_argument("--output", type=Path, help="new artifact directory outside the repository (default: fresh /tmp directory)")
    parser.add_argument("--timeout-seconds", type=float, default=300, help="per-command timeout, 1..3600 seconds (default: 300)")
    args = parser.parse_args(argv)
    if not 1 <= args.timeout_seconds <= 3600:
        parser.error("--timeout-seconds must be between 1 and 3600")
    try:
        output = fresh_output(args.output, ROOT)
        return run_checks(ROOT, output, preflight_only=args.preflight_only, timeout=args.timeout_seconds)
    except (Failure, OSError) as error:
        print(f"[artifacts] {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("[interrupted] diagnostic run stopped", file=sys.stderr)
        sys.exit(130)
