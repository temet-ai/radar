#!/usr/bin/env python3
"""No-build diagnostics tests: disposable fixtures and injected subprocesses only."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("renderer_diagnostics", Path(__file__).with_name("scenario-renderer-diagnostics.py"))
diag = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = diag
spec.loader.exec_module(diag)


class DiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.temp = self.enterContext(tempfile.TemporaryDirectory())
        self.base = Path(self.temp)
        self.root = self.base / "checkout"
        self.fixture = self.root / diag.FIXTURE
        self.fixture.parent.mkdir(parents=True)
        for suffix in (".json", ".txt"):
            shutil.copyfile(diag.ROOT / diag.FIXTURE.with_suffix(suffix), self.fixture.with_suffix(suffix))
        (self.root / "tests").mkdir()
        shutil.copyfile(diag.ROOT / "tests/provider_scenarios.rs", self.root / "tests/provider_scenarios.rs")
        self.out = self.base / "evidence"
        self.out.mkdir()
        self.calls = []
        self.screen = self.enterContext(contextlib.redirect_stdout(io.StringIO()))
        self.enterContext(contextlib.redirect_stderr(self.screen))

    def update_json(self, edit):
        path = self.fixture.with_suffix(".json")
        view = json.loads(path.read_text())
        edit(view)
        path.write_text(json.dumps(view))

    def runner(self, argv, *, cwd, env, timeout):
        self.calls.append((argv, cwd, env, timeout))
        self.assertNotIn("RADAR_SCENARIO_FIXTURES_DIR", env)
        self.assertEqual(env["INSTA_UPDATE"], "no")
        if argv[0] == "npm":
            Path(env["RADAR_SCENARIO_ACTUAL_OUTPUT"]).write_text(self.fixture.with_suffix(".txt").read_text())
        return diag.CommandResult(0, b"fixed command evidence\n")

    def run_check(self, **kwargs):
        return diag.run_checks(self.root, self.out, runner=kwargs.pop("runner", self.runner),
                               dependency_check=kwargs.pop("dependency_check", lambda root, env: None), **kwargs)

    def report(self):
        return json.loads((self.out / "report.json").read_text())

    def assert_preflight_failure(self):
        self.assertEqual(self.run_check(), 1)
        self.assertEqual(self.report()["failed_stage"], "preflight")
        self.assertEqual(self.calls, [])
        self.assertEqual(self.report()["stages"]["producer-freshness"]["status"], "not-run")

    def test_missing_fixture_stops_before_any_process(self):
        self.fixture.with_suffix(".txt").unlink()
        self.assert_preflight_failure()
        self.assertIn("cannot read", self.report()["error"])

    def test_malformed_json_stops_before_any_process(self):
        self.fixture.with_suffix(".json").write_text("{invalid")
        self.assert_preflight_failure()
        self.assertIn("malformed", self.report()["error"])

    def test_non_json_numbers_are_rejected(self):
        path = self.fixture.with_suffix(".json")
        path.write_text(path.read_text().replace('"width": 120', '"width": NaN'))
        self.assert_preflight_failure()
        self.assertIn("malformed", self.report()["error"])

    def test_special_file_input_is_rejected_without_opening_it(self):
        path = self.fixture.with_suffix(".json")
        path.unlink()
        os.mkfifo(path)
        self.assert_preflight_failure()
        self.assertIn("regular file", self.report()["error"])

    def test_non_object_json_is_rejected(self):
        self.fixture.with_suffix(".json").write_text("[]")
        self.assert_preflight_failure()

    def test_dimensions_are_fixed(self):
        self.update_json(lambda view: view.update(width=80))
        self.assert_preflight_failure()
        self.assertIn("dimensions", self.report()["error"])

    def test_boolean_dimensions_are_not_integers(self):
        self.update_json(lambda view: view.update(width=True))
        self.assert_preflight_failure()

    def test_chrome_dimensions_are_checked(self):
        self.update_json(lambda view: view["chrome"].update(height=24))
        self.assert_preflight_failure()

    def test_basic_shape_is_checked(self):
        self.update_json(lambda view: view["screen"].update(panes={}))
        self.assert_preflight_failure()

    def test_wrong_semantic_screen_is_rejected(self):
        self.update_json(lambda view: view["screen"].update(scope="jira"))
        self.assert_preflight_failure()

    def test_json_marker_mismatch_is_rejected(self):
        path = self.fixture.with_suffix(".json")
        path.write_text(path.read_text().replace(diag.MARKER, "A different patrol"))
        self.assert_preflight_failure()
        self.assertIn("marker", self.report()["error"])

    def test_text_marker_mismatch_is_rejected(self):
        path = self.fixture.with_suffix(".txt")
        path.write_text(path.read_text().replace(diag.MARKER, "A different patrol"))
        self.assert_preflight_failure()

    def test_text_row_count_is_checked_without_claiming_cell_width(self):
        path = self.fixture.with_suffix(".txt")
        path.write_text(path.read_text() + "\nextra row")
        self.assert_preflight_failure()
        self.assertIn("display-cell widths are not checked", self.report()["error"])

    def test_large_inputs_are_rejected(self):
        self.fixture.with_suffix(".json").write_bytes(b"x" * (diag.INPUT_BYTES + 1))
        self.assert_preflight_failure()

    def test_invalid_utf8_fixture_fails_preflight(self):
        self.fixture.with_suffix(".txt").write_bytes(b"\xff")
        self.assert_preflight_failure()
        self.assertIn("cannot read", self.report()["error"])

    def test_preflight_only_is_explicitly_not_qualified(self):
        with patch.object(diag, "check_dependencies", side_effect=AssertionError("should not run")):
            self.assertEqual(self.run_check(preflight_only=True), 0)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.report()["status"], "preflight-only")
        self.assertIn("NOT QUALIFIED", self.report()["qualification"])
        self.assertIn("no full protocol validation", self.report()["stages"]["preflight"]["evidence"])

    def test_missing_dependencies_fail_before_cargo(self):
        with patch.object(diag.shutil, "which", return_value=None):
            self.assertEqual(self.run_check(dependency_check=diag.check_dependencies), 1)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.report()["failed_stage"], "dependencies")
        self.assertIn("vitest", self.report()["error"])

    def test_success_uses_fixed_recipe_in_order_and_real_capture(self):
        before = {suffix: self.fixture.with_suffix(suffix).read_bytes() for suffix in (".json", ".txt")}
        self.assertEqual(self.run_check(environ={"RADAR_SCENARIO_FIXTURES_DIR": "bad", "INSTA_UPDATE": "always",
                                               "RADAR_SCENARIO_ACTUAL_OUTPUT": "bad"}), 0)
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.calls[0][0], [sys.executable, str(self.root / "scripts/scenarios"), "github-denied-detail"])
        self.assertNotIn("RADAR_SCENARIO_ACTUAL_OUTPUT", self.calls[0][2])
        self.assertEqual(self.calls[1][0], ["npm", "test", "--", "--maxWorkers=1", "test/scenarios/checkpoints.test.tsx"])
        self.assertEqual(self.calls[1][1], self.root / "ink")
        report = self.report()
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["dimensions"], {"width": 120, "height": 40})
        self.assertEqual(len(report["fixtures"]), 2)
        self.assertEqual(len(report["producer_source_sha256"]), 64)
        self.assertIn("styles", report["scope"])
        self.assertEqual((self.out / "expected.normalized.txt").read_bytes(),
                         (self.out / "actual.normalized.txt").read_bytes())
        for suffix, contents in before.items():
            self.assertEqual(self.fixture.with_suffix(suffix).read_bytes(), contents)

    def test_stale_producer_refuses_to_run_ink_even_when_two_fixtures_agree(self):
        def stale(argv, **kwargs):
            self.calls.append(argv)
            return diag.CommandResult(101, b"scenario fixture drift: github_refresh_120x40.json")
        self.assertEqual(self.run_check(runner=stale), 101)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.report()["failed_stage"], "producer-freshness")
        self.assertEqual(self.report()["stages"]["ink"]["status"], "not-run")
        self.assertIn("fixture drift", (self.out / "producer-freshness.log").read_text())

    def test_fixture_changed_during_producer_refuses_ink(self):
        def changing(argv, **kwargs):
            self.calls.append(argv)
            path = self.fixture.with_suffix(".txt")
            path.write_text(path.read_text().replace("Brass", "Brass?"))
            return diag.CommandResult(0, b"producer passed")
        self.assertEqual(self.run_check(runner=changing), 1)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.report()["failed_stage"], "producer-freshness")
        self.assertIn("changed since preflight", self.report()["error"])

    def test_fixture_changed_during_ink_refuses_qualification(self):
        def changing(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if argv[0] == "npm":
                path = self.fixture.with_suffix(".json")
                path.write_text(path.read_text() + "\n")
            return result
        self.assertEqual(self.run_check(runner=changing), 1)
        self.assertEqual(self.report()["failed_stage"], "ink")
        self.assertIn("changed since preflight", self.report()["error"])
        self.assertEqual(self.report()["status"], "failed")

    def test_subprocess_error_names_stage(self):
        def missing(*args, **kwargs):
            raise FileNotFoundError("test executable unavailable")
        self.assertEqual(self.run_check(runner=missing), 1)
        self.assertEqual(self.report()["failed_stage"], "producer-freshness")

    def test_timeout_stops_downstream_and_preserves_signal_exit(self):
        def timeout(*args, **kwargs):
            self.calls.append(args)
            return diag.CommandResult(-9, b"still compiling", timed_out=True)
        self.assertEqual(self.run_check(runner=timeout, timeout=2), 124)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.report()["stages"]["producer-freshness"]["exit_code"], -9)
        self.assertTrue(self.report()["stages"]["producer-freshness"]["timed_out"])

    def test_missing_actual_is_never_reported_as_pass(self):
        def absent(*args, **kwargs):
            return diag.CommandResult(0, b"0 tests passed")
        self.assertEqual(self.run_check(runner=absent), 1)
        self.assertEqual(self.report()["failed_stage"], "ink")
        self.assertIn("no actual frame", self.report()["error"])
        self.assertFalse((self.out / "actual.normalized.txt").exists())

    def test_mismatch_captures_actual_and_bounded_context_even_if_ink_fails(self):
        def mismatch(argv, **kwargs):
            if argv[0] == "npm":
                Path(kwargs["env"]["RADAR_SCENARIO_ACTUAL_OUTPUT"]).write_text("wrong frame\n")
                return diag.CommandResult(7, b"Ink assertion failed")
            return self.runner(argv, **kwargs)
        self.assertEqual(self.run_check(runner=mismatch), 7)
        report = self.report()
        self.assertEqual(report["failed_stage"], "ink")
        self.assertEqual(report["stages"]["text-parity"]["status"], "failed")
        self.assertEqual(report["first_difference"]["row"], 1)
        self.assertEqual((self.out / "actual.normalized.txt").read_text(), "wrong frame\n")

    def test_mismatch_fails_even_if_subprocess_reports_success(self):
        def mismatch(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if argv[0] == "npm":
                Path(kwargs["env"]["RADAR_SCENARIO_ACTUAL_OUTPUT"]).write_text("wrong frame")
            return result
        self.assertEqual(self.run_check(runner=mismatch), 1)
        self.assertEqual(self.report()["failed_stage"], "text-parity")

    def test_logs_are_bounded(self):
        def noisy(*args, **kwargs):
            return diag.CommandResult(1, b"discarded-prefix" + b"x" * (diag.LOG_BYTES + 100))
        self.assertEqual(self.run_check(runner=noisy), 1)
        log = (self.out / "producer-freshness.log").read_bytes()
        self.assertEqual(len(log), diag.LOG_BYTES)
        self.assertNotIn(b"discarded-prefix", log)

    def test_invalid_utf8_log_is_still_bounded_after_replacement(self):
        def noisy(*args, **kwargs):
            return diag.CommandResult(1, b"\xff" * diag.LOG_BYTES)
        self.assertEqual(self.run_check(runner=noisy), 1)
        log = (self.out / "producer-freshness.log").read_bytes()
        self.assertLessEqual(len(log), diag.LOG_BYTES)
        log.decode("utf-8")

    def test_artifact_collision_does_not_overwrite(self):
        (self.out / "expected.normalized.txt").write_text("caller-owned")
        self.assertEqual(self.run_check(), 1)
        self.assertEqual(self.report()["failed_stage"], "artifacts")
        self.assertEqual((self.out / "expected.normalized.txt").read_text(), "caller-owned")
        self.assertEqual(self.calls, [])

    def test_invalid_utf8_actual_capture_is_not_qualified(self):
        def invalid(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if argv[0] == "npm":
                Path(kwargs["env"]["RADAR_SCENARIO_ACTUAL_OUTPUT"]).write_bytes(b"\xff")
            return result
        self.assertEqual(self.run_check(runner=invalid), 1)
        self.assertEqual(self.report()["failed_stage"], "ink")
        self.assertIn("cannot read actual.ansi.txt", self.report()["error"])
        self.assertIn("NOT QUALIFIED", self.report()["qualification"])

    def test_actual_symlink_is_refused(self):
        victim = self.base / "victim"
        victim.write_text("do not alter")
        def symlink(argv, **kwargs):
            if argv[0] == "npm":
                Path(kwargs["env"]["RADAR_SCENARIO_ACTUAL_OUTPUT"]).symlink_to(victim)
            return diag.CommandResult(0, b"done")
        self.assertEqual(self.run_check(runner=symlink), 1)
        self.assertIn("symlink", self.report()["error"])
        self.assertEqual(victim.read_text(), "do not alter")

    def test_fresh_output_refuses_existing_inside_repo_missing_parent_and_symlinks(self):
        link = self.base / "link"
        link.symlink_to(self.base, target_is_directory=True)
        for path in (self.out, self.root / "evidence", self.base / "absent/evidence", link / "evidence",
                     self.base / "unused/../evidence"):
            with self.subTest(path=path), self.assertRaises(diag.Failure):
                diag.fresh_output(path, self.root)
        created = diag.fresh_output(self.base / "fresh", self.root)
        self.assertTrue(created.is_dir())
        self.assertEqual(created.stat().st_mode & 0o777, 0o700)

    def test_default_artifacts_are_external_even_with_ambient_tmpdir(self):
        with patch.dict(os.environ, {"TMPDIR": str(self.root)}):
            output = diag.fresh_output(None, self.root)
        try:
            self.assertNotIn(self.root, output.parents)
            self.assertEqual(output.stat().st_uid, os.getuid())
        finally:
            output.rmdir()

    def test_default_path_is_canonical_when_tmp_is_a_symlink(self):
        link = self.base / "tmp-alias"
        link.symlink_to(self.base, target_is_directory=True)
        (self.base / "fresh-default").mkdir()
        with patch.object(diag.tempfile, "mkdtemp", return_value=str(link / "fresh-default")):
            output = diag.fresh_output(None, self.root)
        self.assertEqual(output, self.base / "fresh-default")

    def test_normalization_matches_checkpoint_contract(self):
        self.assertEqual(diag.normalize("\x1b[31mred\x1b[0m  \nword\u00a0\n\n"), "red\nword")
        self.assertEqual(diag.normalize("left  middle\n  inset"), "left  middle\n  inset")

    def test_diff_is_bounded_and_uses_codepoints_not_cells(self):
        self.assertIsNone(diag.text_diff("same", "same"))
        diff = diag.text_diff("界e\u0301x", "界e\u0301y")
        self.assertEqual(diff["codepoint_column"], 4)
        self.assertIn("not display cells", diff["coordinate_unit"])
        left = "\n".join(["a" * 2000] * 100)
        right = left.replace("a", "b")
        diff = diag.text_diff(left, right)
        self.assertLessEqual(len(diff["context"]), 5)
        for row in diff["context"]:
            self.assertLessEqual(len(row["expected"]), 120)
            self.assertLessEqual(len(row["actual"]), 120)
        self.assertEqual(diag.text_diff("one\ntwo", "one")["row"], 2)
        self.assertEqual(diag.text_diff("abc", "abcd")["codepoint_column"], 4)

    def test_real_subprocess_captures_bounded_tail_without_shell(self):
        result = diag.run_process([sys.executable, "-c", "print('x' * 40000)"],
                                  cwd=self.base, env=dict(os.environ), timeout=3)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(len(result.output), diag.LOG_BYTES)
        self.assertFalse(result.timed_out)

    def test_real_timeout_kills_and_reaps_process_group(self):
        # Descendant would otherwise outlive the parent and keep stdout open.
        source = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); print('started',flush=True); time.sleep(60)"
        result = diag.run_process([sys.executable, "-c", source], cwd=self.base,
                                  env=dict(os.environ), timeout=0.2)
        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, -9)
        self.assertIn(b"started", result.output)

    def test_timeout_is_bounded_after_child_closes_stdout(self):
        result = diag.run_process([sys.executable, "-c", "import os,time; os.close(1); os.close(2); time.sleep(60)"],
                                  cwd=self.base, env=dict(os.environ), timeout=0.2)
        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, -9)

    def test_cli_rejects_arbitrary_commands_and_bad_timeout(self):
        for argv in (["--command", "true"], ["--timeout-seconds", "0"], ["--timeout-seconds", "nan"]):
            with self.subTest(argv=argv), self.assertRaises(SystemExit) as raised:
                diag.main(argv)
            self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
