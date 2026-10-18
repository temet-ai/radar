#!/usr/bin/env python3
"""No-build checks for the headless launcher, using only the Python stdlib."""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
path = Path(__file__).with_name("scenarios")
loader = importlib.machinery.SourceFileLoader("scenario_launcher", str(path))
spec = importlib.util.spec_from_loader(loader.name, loader)
launcher = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = launcher
loader.exec_module(launcher)
helper_loader = importlib.machinery.SourceFileLoader("scenario_supervisor", str(launcher.SUPERVISOR))
helper_spec = importlib.util.spec_from_loader(helper_loader.name, helper_loader)
helper = importlib.util.module_from_spec(helper_spec)
helper_loader.exec_module(helper)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.output = io.StringIO()
        self.stack.enter_context(contextlib.redirect_stdout(self.output))
        self.stack.enter_context(contextlib.redirect_stderr(self.output))

    def test_list_never_runs_cargo(self):
        with patch.object(launcher.subprocess, "Popen") as run:
            self.assertEqual(launcher.main(["--list"]), 0)
        run.assert_not_called()
        for scenario in launcher.SCENARIOS:
            self.assertIn(scenario.name, self.output.getvalue())
            self.assertIn(scenario.description, self.output.getvalue())

    def test_exact_commands_match_real_test_functions(self):
        for scenario in launcher.SCENARIOS:
            argv = launcher.command(scenario)
            source = (launcher.ROOT / f"tests/{scenario.binary}.rs").read_text()
            function = scenario.test.split("::")[-1]
            self.assertRegex(source, rf"(?:async )?fn {function}\(\)")
            if "::" in scenario.test:
                self.assertIn(f"mod {scenario.test.split('::')[0]} {{", source)
            self.assertEqual(argv, [
                "cargo", "test", "--locked", "--offline", "--color", "never",
                "--target-dir", str(launcher.ROOT / "target"),
                "-p", "radar", "--no-default-features", "--features", f"{scenario.feature},test-support",
                "--test", scenario.binary, "--", "--exact", scenario.test,
                "--nocapture", "--test-threads=1",
            ])
            self.assertIs(launcher.select(scenario.name), scenario)

    def test_provider_mappings_select_only_their_parts_and_test_support(self):
        expected = {
            "slack-success": ("slack", "slack_scenarios"),
            "slack-refusal": ("slack", "slack_scenarios"),
            "slack-uncertain": ("slack", "slack_scenarios"),
            "slack-lost-response": ("slack", "slack_scenarios"),
            "github-denied-detail": ("github", "provider_scenarios"),
            "gitlab-refresh": ("gitlab", "provider_scenarios"),
            "jira-partial-refresh": ("jira", "provider_scenarios"),
            "git-work-link": ("git,workbench", "local_scenarios"),
            "agent-permissions": ("sessions", "agent_scenarios"),
            "review-no-worktree": ("gitlab,sessions", "review_scenarios"),
        }
        self.assertEqual({s.name: (s.feature, s.binary) for s in launcher.SCENARIOS}, expected)
        for name, (parts, binary) in expected.items():
            argv = launcher.command(launcher.select(name))
            self.assertEqual(argv[argv.index("--features") + 1], f"{parts},test-support")
            self.assertEqual(argv[argv.index("--test") + 1], binary)
            with patch.object(launcher.subprocess, "Popen") as run:
                self.assertEqual(launcher.main([name, "--dry-run"]), 0)
            run.assert_not_called()

    def test_cargo_path_is_one_argument(self):
        argv = launcher.command(launcher.SCENARIOS[0], "./tool chain/cargo")
        self.assertEqual(argv[0], str(Path("./tool chain/cargo").resolve()))

    def test_unknown_and_incompatible_arguments_fail_before_build(self):
        cases = [[], ["no-such-scenario"], ["--list", "slack-success"],
                 ["--interactive", "slack-success"], ["--all-features"],
                 ["github-denied-detail", "--features", "github,slack"],
                 ["gitlab-refresh", "--test", "mix"]]
        with patch.object(launcher.subprocess, "Popen") as run:
            for args in cases:
                with self.subTest(args=args), self.assertRaises(SystemExit) as raised:
                    launcher.main(args)
                self.assertEqual(raised.exception.code, 2)
        run.assert_not_called()

    def test_dry_run_selects_without_build(self):
        with patch.object(launcher.subprocess, "Popen") as run:
            self.assertEqual(launcher.main(["slack-refusal", "--dry-run"]), 0)
        run.assert_not_called()
        self.assertIn(launcher.SCENARIOS[1].test, self.output.getvalue())

    def test_interactive_parser(self):
        for index, scenario in enumerate(launcher.SCENARIOS, 1):
            self.assertIs(launcher.interactive_selection(str(index)), scenario)
            self.assertIs(launcher.interactive_selection(f" {scenario.name} "), scenario)
        for value in ("r", "replay", ""):
            with self.assertRaises(ValueError):
                launcher.interactive_selection(value)
            self.assertIs(launcher.interactive_selection(value, launcher.SCENARIOS[0]),
                          launcher.SCENARIOS[0])
        for value in ("0", str(len(launcher.SCENARIOS) + 1), "--ignored", "²"):
            with self.assertRaises(ValueError):
                launcher.interactive_selection(value)
        self.assertIsNone(launcher.interactive_selection("q"))

    def test_interactive_replay_and_failure_are_not_hidden(self):
        with patch("builtins.input", side_effect=["bad", "1", "r", "q"]), \
             patch.object(launcher, "run_scenario", side_effect=[1, 0]) as run:
            self.assertEqual(launcher.main(["--interactive"]), 1)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0], run.call_args_list[1])

    def test_interactive_eof_quits(self):
        with patch("builtins.input", side_effect=EOFError):
            self.assertEqual(launcher.main(["--interactive"]), 0)

    def test_interactive_interrupt_exits_without_another_prompt(self):
        for signum in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signum=signum), patch("builtins.input", side_effect=["1"]) as prompt, \
                 patch.object(launcher, "run_scenario", return_value=128 + signum):
                self.assertEqual(launcher.main(["--interactive"]), 128 + signum)
                self.assertEqual(prompt.call_count, 1)

    def phases(self, output, code=0, build_output="", build_code=0):
        return [launcher.PhaseResult(build_code, build_output),
                launcher.PhaseResult(code, output)]

    def test_compile_only_then_exact_execution_with_fixed_budgets(self):
        scenario = launcher.SCENARIOS[0]
        with patch.object(launcher, "run_phase", side_effect=self.phases(PASS)) as run:
            self.assertEqual(launcher.run_scenario(scenario), 0)
        self.assertEqual(run.call_args_list[0].args,
                         (launcher.build_command(scenario), 900))
        self.assertEqual(run.call_args_list[1].args,
                         (launcher.command(scenario), 20))
        for scenario in launcher.SCENARIOS:
            argv = launcher.command(scenario)
            self.assertEqual(launcher.build_command(scenario),
                             argv[:argv.index("--")] + ["--no-run"])

    def test_single_pass_is_required_only_from_execution(self):
        for output, expected in [
            (PASS, 0),
            (ZERO, 1),
            ("test result: ok. 0 passed; 0 failed; 1 ignored; 2 filtered out\n", 1),
            ("test result: ok. 2 passed; 0 failed; 0 ignored; 1 filtered out\n", 1),
            ("", 1),
            (PASS + ZERO, 1),
        ]:
            with self.subTest(output=output), patch.object(
                    launcher, "run_phase", side_effect=self.phases(output, build_output=PASS)):
                self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), expected)

    def test_build_failure_stops_before_execution(self):
        with patch.object(launcher, "run_phase", return_value=
                          launcher.PhaseResult(101, "compile failed\n")) as run:
            self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), 101)
        self.assertEqual(run.call_count, 1)
        self.assertIn("Build failed (Cargo exit 101)", self.output.getvalue())

    def test_execution_failure_preserves_status_even_with_pass_output(self):
        for code, expected in [(101, 101), (-signal.SIGABRT, 1)]:
            with self.subTest(code=code), patch.object(
                    launcher, "run_phase", side_effect=self.phases(PASS, code)):
                self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), expected)
        self.assertIn("Execution failed", self.output.getvalue())

    def test_missing_cargo_is_actionable(self):
        with patch.object(launcher.subprocess, "Popen", side_effect=FileNotFoundError("cargo")):
            self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), 1)
        self.assertIn("--cargo /path/to/cargo", self.output.getvalue())
        self.assertIn("during build", self.output.getvalue())

    def test_unsupported_platform_fails_closed_but_can_list_and_dry_run(self):
        with patch.object(launcher.os, "name", "nt"), \
             patch.object(launcher.subprocess, "Popen") as run:
            self.assertEqual(launcher.main(["slack-success"]), 1)
            self.assertEqual(launcher.main(["--list"]), 0)
            self.assertEqual(launcher.main(["slack-success", "--dry-run"]), 0)
        run.assert_not_called()
        self.assertIn("requires POSIX process groups", self.output.getvalue())

    def test_timeout_cannot_be_disabled_or_changed_on_cli(self):
        with patch.object(launcher.subprocess, "Popen") as run:
            for option in ("--timeout", "--build-timeout", "--no-timeout"):
                with self.subTest(option=option), self.assertRaises(SystemExit):
                    launcher.main(["slack-success", option, "0"])
        run.assert_not_called()

    def test_missing_required_capability_custom_reaper_or_threads_fail_before_spawn(self):
        with patch.object(launcher.subprocess, "Popen") as run:
            with patch.object(launcher.os, "pipe", create=True):
                del launcher.os.pipe
                self.assertEqual(launcher.main(["slack-success"]), 1)
            for handler in (signal.SIG_IGN, lambda *_: None):
                with self.subTest(handler=handler), patch.object(
                        launcher.signal, "getsignal", return_value=handler):
                    self.assertEqual(launcher.main(["slack-success"]), 1)
            with patch.object(launcher.threading, "active_count", return_value=2):
                self.assertEqual(launcher.main(["slack-success"]), 1)
            with patch.object(launcher.threading, "current_thread", return_value=object()):
                self.assertEqual(launcher.main(["slack-success"]), 1)
        run.assert_not_called()

    def test_cleanup_failure_cannot_report_success(self):
        with patch.object(launcher, "run_phase", return_value=launcher.PhaseResult(
                0, PASS, cleanup_failed=True)) as run:
            self.assertEqual(launcher.main(["slack-success"]), 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn("cleanup failed", self.output.getvalue())

    @contextlib.contextmanager
    def fake_supervisor(self, report=(0, False, "")):
        with contextlib.ExitStack() as stack:
            spawn = stack.enter_context(patch.object(launcher.subprocess, "Popen"))
            process = spawn.return_value
            process.pid = 123456
            process.returncode = None
            events = []
            kill = stack.enter_context(patch.object(launcher.os, "killpg",
                side_effect=lambda pid, sig: events.append((pid, sig))))
            reap = stack.enter_context(patch.object(launcher.os, "waitpid",
                side_effect=lambda pid, flags: (events.append("reap") or (pid, 0))))
            sleep = stack.enter_context(patch.object(launcher.time, "sleep"))
            observe = stack.enter_context(patch.object(launcher, "observe_report", return_value=report))
            yield process, events, spawn, kill, reap, sleep, observe
            for method in ("poll", "wait", "communicate", "send_signal", "terminate", "kill", "__enter__", "__exit__"):
                getattr(process, method).assert_not_called()

    def test_reserved_supervisor_signals_precede_the_only_raw_reap(self):
        for report in ((0, False, ""), (73, False, ""), (1, True, ""), (1, False, "bad report")):
            with self.subTest(report=report), self.fake_supervisor(report) as controls:
                process, events, spawn, _, reap, _, _ = controls
                result = launcher.run_phase(["cargo with spaces", "--exact", "name"], 2)
                self.assertEqual(result.returncode, report[0])
                self.assertEqual(result.timed_out, report[1])
                self.assertEqual(result.error, report[2])
                self.assertFalse(result.cleanup_failed)
                self.assertEqual(events, [(123456, signal.SIGTERM), (123456, signal.SIGKILL), "reap"])
                reap.assert_called_once_with(123456, os.WNOHANG)
                args, kwargs = spawn.call_args
                argv = args[0]
                self.assertIsInstance(argv, list)
                self.assertEqual(argv[:4], [str(Path(sys.executable).resolve()), "-I", "-S", str(launcher.SUPERVISOR)])
                self.assertEqual(argv[5:], ["--", "cargo with spaces", "--exact", "name"])
                self.assertEqual(kwargs["pass_fds"], (int(argv[4]),))
                self.assertEqual(kwargs["cwd"], launcher.ROOT)
                self.assertTrue(kwargs["start_new_session"])
                self.assertTrue(kwargs["close_fds"])
                self.assertNotIn("env", kwargs)
                self.assertNotIn("shell", kwargs)
                self.assertEqual(process.returncode, 0)

    def test_known_ownership_loss_before_or_between_signals_never_resignals(self):
        for moment in ("before", "between"):
            for cause in ("returncode", "handler"):
                with self.subTest(moment=moment, cause=cause), self.fake_supervisor() as controls:
                    process, events, _, kill, reap, _, observe = controls
                    default = signal.getsignal(signal.SIGCHLD)
                    current = [default]

                    def lose():
                        if cause == "returncode":
                            process.returncode = 0
                        else:
                            current[0] = signal.SIG_IGN

                    if moment == "before":
                        observe.side_effect = lambda *args: (lose() or (0, False, ""))
                    else:
                        kill.side_effect = lambda pid, sig: (events.append((pid, sig)), lose())
                    real_getsignal = signal.getsignal
                    with patch.object(signal, "getsignal", side_effect=lambda sig:
                                      current[0] if sig == signal.SIGCHLD else real_getsignal(sig)):
                        result = launcher.run_phase(["cargo"], 2)
                    self.assertTrue(result.cleanup_failed)
                    self.assertEqual(kill.call_count, 0 if moment == "before" else 1)
                    reap.assert_not_called()
                    self.assertIsNotNone(process.returncode)

    def test_echild_during_final_reap_is_failure_without_further_signals(self):
        with self.fake_supervisor() as controls:
            process, events, _, kill, reap, _, _ = controls
            reap.side_effect = ChildProcessError("not our child")
            result = launcher.run_phase(["cargo"], 2)
            self.assertTrue(result.cleanup_failed)
            self.assertIn("reap failed", result.cleanup_error)
            self.assertEqual(kill.call_count, 2)
            self.assertEqual(process.returncode, launcher.ABANDONED_RETURN_CODE)

    def test_first_interrupt_survives_spawn_grace_and_final_reap(self):
        original = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        with self.fake_supervisor() as controls:
            process, events, spawn, _, reap, sleep, _ = controls

            def launch(*args, **kwargs):
                signal.raise_signal(signal.SIGINT)
                return process

            def final_reap(pid, flags):
                signal.raise_signal(signal.SIGTERM)
                events.append("reap")
                return pid, 0

            spawn.side_effect = launch
            sleep.side_effect = lambda _: signal.raise_signal(signal.SIGTERM)
            reap.side_effect = final_reap
            result = launcher.run_phase(["cargo"], 2)
            self.assertEqual(result.interrupted, signal.SIGINT)
            self.assertEqual(events, [(123456, signal.SIGTERM), (123456, signal.SIGKILL), "reap"])
        self.assertEqual({sig: signal.getsignal(sig) for sig in original}, original)

    def test_final_reap_timeout_is_bounded(self):
        with self.fake_supervisor() as controls:
            process, _, _, kill, reap, _, _ = controls
            reap.return_value = (0, 0)
            reap.side_effect = None
            with patch.object(launcher.time, "monotonic", side_effect=[0, 0, 2]):
                result = launcher.run_phase(["cargo"], 2)
            self.assertTrue(result.cleanup_failed)
            self.assertIn("reap exceeded", result.cleanup_error)
            self.assertEqual(kill.call_count, 2)
            self.assertEqual(process.returncode, launcher.ABANDONED_RETURN_CODE)

    def test_observer_error_still_cleans_reserved_group(self):
        with self.fake_supervisor() as controls:
            _, events, _, _, _, _, observe = controls
            observe.side_effect = OSError("read failure")
            result = launcher.run_phase(["cargo"], 2)
            self.assertIn("read failure", result.error)
            self.assertEqual(events, [(123456, signal.SIGTERM), (123456, signal.SIGKILL), "reap"])

    def test_supervisor_startup_consumes_the_original_phase_deadline(self):
        observe_report = launcher.observe_report
        with self.fake_supervisor() as controls:
            _, _, _, _, _, _, observe = controls
            observe.side_effect = observe_report
            with patch.object(launcher.time, "monotonic", side_effect=[0, 5, 5]):
                result = launcher.run_phase(["cargo"], 2)
            self.assertTrue(result.timed_out)

    def test_pipe_creation_failure_never_spawns(self):
        with patch.object(os, "pipe", side_effect=OSError("pipe unavailable")), \
             patch.object(launcher.subprocess, "Popen") as spawn:
            self.assertEqual(launcher.main(["slack-success"]), 1)
        spawn.assert_not_called()
        self.assertIn("pipe unavailable", self.output.getvalue())

    def test_fixed_report_protocol_requires_exact_record_and_valid_values(self):
        valid = launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, 0)
        bad = [b"", valid[:3], valid + b"x", valid * 2,
               launcher.REPORT.pack(b"BAD!", 1, 0),
               launcher.REPORT.pack(launcher.REPORT_MAGIC, 4, 0),
               launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, 256),
               launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, -128),
               launcher.REPORT.pack(launcher.REPORT_MAGIC, 2, 0),
               launcher.REPORT.pack(launcher.REPORT_MAGIC, 3, 0)]
        for data in bad:
            with self.subTest(data=data):
                self.assertTrue(launcher.decode_report(data)[1])
        for code in (0, 73, -signal.SIGTERM):
            self.assertEqual(launcher.decode_report(launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, code)),
                             (code, ""))
        self.assertIn("Could not launch", launcher.decode_report(
            launcher.REPORT.pack(launcher.REPORT_MAGIC, 2, 2))[1])
        self.assertIn("internal error", launcher.decode_report(
            launcher.REPORT.pack(launcher.REPORT_MAGIC, 3, 1))[1])

    def test_report_buffer_is_bounded_and_eof_is_required_before_acceptance(self):
        valid = launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, 0)
        for chunks, expected_error in [([valid, b""], False), ([valid + b"x"], True),
                                       ([valid, b"x"], True), ([valid[:2], b""], True)]:
            with self.subTest(chunks=chunks), patch.object(os, "read", side_effect=chunks) as read, \
                 patch.object(launcher.time, "monotonic", return_value=0):
                code, timeout, error = launcher.observe_report(123, 1, [])
                self.assertFalse(timeout)
                self.assertEqual(bool(error), expected_error)
                self.assertLessEqual(max(call.args[1] for call in read.call_args_list), launcher.REPORT.size + 1)
                if len(chunks) == 2 and len(chunks[0]) == launcher.REPORT.size:
                    self.assertEqual(read.call_args_list[-1].args[1], 1)
        with patch.object(os, "read", side_effect=[valid, BlockingIOError]), \
             patch.object(launcher.time, "sleep"), \
             patch.object(launcher.time, "monotonic", side_effect=[0, 0, 2]):
            self.assertEqual(launcher.observe_report(123, 1, []), (1, True, ""))

    def test_status_ready_after_deadline_is_not_accepted(self):
        valid = launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, 0)
        with patch.object(os, "read", side_effect=[valid, b""]), \
             patch.object(launcher.time, "monotonic", side_effect=[0, 0, 2]):
            self.assertEqual(launcher.observe_report(123, 1, []), (1, True, ""))

    def test_failed_spawn_and_pipe_setup_close_both_descriptors(self):
        real_pipe = os.pipe
        for failure in ("spawn", "setup"):
            descriptors = []

            def pipe():
                ends = real_pipe()
                descriptors.extend(ends)
                return ends

            with self.subTest(failure=failure), patch.object(os, "pipe", side_effect=pipe), \
                 patch.object(launcher.subprocess, "Popen", side_effect=FileNotFoundError("helper")) as spawn:
                with patch.object(os, "set_blocking", side_effect=OSError("setup")) if failure == "setup" else contextlib.nullcontext():
                    with self.assertRaises(OSError):
                        launcher.run_phase(["cargo"], 2)
            for descriptor in descriptors:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)
            if failure == "setup":
                spawn.assert_not_called()

    def test_missing_helper_or_interpreter_never_spawns(self):
        with patch.object(launcher.subprocess, "Popen") as spawn:
            with patch.object(launcher, "SUPERVISOR", Path("/missing/watch-helper")):
                with self.assertRaises(FileNotFoundError):
                    launcher.run_phase(["cargo"], 2)
            with patch.object(sys, "executable", "/missing/watch-python"):
                with self.assertRaises(FileNotFoundError):
                    launcher.run_phase(["cargo"], 2)
        spawn.assert_not_called()


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.output = io.StringIO()
        self.stack.enter_context(contextlib.redirect_stderr(self.output))
        self.stack.enter_context(patch.object(helper.os, "getpid", return_value=2468))
        self.stack.enter_context(patch.object(helper.os, "getpgrp", return_value=2468))
        self.stack.enter_context(patch.object(helper.os, "getsid", return_value=2468))

    def test_cargo_has_original_arguments_inherited_environment_and_no_status_fd(self):
        with patch.object(helper.os, "set_inheritable") as inheritable, \
             patch.object(helper.subprocess, "Popen") as spawn, \
             patch.object(helper.os, "waitpid", return_value=(4321, 73 << 8)) as wait, \
             patch.object(helper, "report_and_wait", return_value=0) as report:
            cargo = spawn.return_value
            cargo.pid = 4321
            self.assertEqual(helper.main(["17", "--", "/tool chain/cargo", "--exact", "a b"]), 0)
        inheritable.assert_called_once_with(17, False)
        spawn.assert_called_once_with(["/tool chain/cargo", "--exact", "a b"], close_fds=True, pass_fds=())
        wait.assert_called_once_with(4321, 0)
        report.assert_called_once_with(17, 1, 73)
        self.assertEqual(cargo.returncode, 73)
        cargo.poll.assert_not_called()
        cargo.wait.assert_not_called()

    def test_missing_cargo_and_lost_wait_ownership_never_report_success(self):
        with patch.object(helper.os, "set_inheritable"), \
             patch.object(helper.subprocess, "Popen", side_effect=FileNotFoundError(2, "missing")), \
             patch.object(helper, "report_and_wait", return_value=1) as report:
            self.assertEqual(helper.main(["17", "--", "cargo"]), 1)
        report.assert_called_once_with(17, 2, 2)
        with patch.object(helper.os, "set_inheritable"), \
             patch.object(helper.subprocess, "Popen") as spawn, \
             patch.object(helper.os, "waitpid", side_effect=ChildProcessError("lost")), \
             patch.object(helper, "report_and_wait", return_value=1) as report:
            self.assertEqual(helper.main(["17", "--", "cargo"]), 1)
            self.assertEqual(spawn.return_value.returncode, helper.ABANDONED_RETURN_CODE)
        report.assert_called_once_with(17, 3, 1)

    def test_malformed_invocation_or_unowned_session_never_launches(self):
        with patch.object(helper.subprocess, "Popen") as spawn:
            for args in ([], ["1", "--", "cargo"], ["²", "--", "cargo"], ["5", "cargo"],
                         ["5", "bad", "cargo"], ["-1", "--", "cargo"]):
                with self.subTest(args=args):
                    self.assertEqual(helper.main(args), 1)
            with patch.object(helper.os, "set_inheritable"), \
                 patch.object(helper.os, "getpgrp", return_value=100), \
                 patch.object(helper, "report_and_wait", return_value=1) as report:
                self.assertEqual(helper.main(["17", "--", "cargo"]), 1)
            report.assert_called_once_with(17, 3, 1)
        spawn.assert_not_called()

    def test_status_write_is_single_fixed_record_and_closes_before_waiting(self):
        events = []
        with patch.object(helper.os, "write", side_effect=lambda fd, data:
                          (events.append((fd, data)) or len(data))) as write, \
             patch.object(helper.os, "close", side_effect=lambda fd: events.append("closed")), \
             patch.object(helper.signal, "pause", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                helper.report_and_wait(17, 1, -signal.SIGTERM)
        self.assertEqual(write.call_count, 1)
        self.assertEqual(events, [(17, launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, -signal.SIGTERM)), "closed"])
        self.assertEqual(helper.REPORT.size, 9)

    def test_status_short_write_or_broken_pipe_closes_and_fails(self):
        for outcome in (3, BrokenPipeError("closed reader")):
            with self.subTest(outcome=outcome), patch.object(helper.os, "write") as write, \
                 patch.object(helper.os, "close") as close, patch.object(helper.signal, "pause") as pause:
                if isinstance(outcome, Exception):
                    write.side_effect = outcome
                else:
                    write.return_value = outcome
                self.assertEqual(helper.report_and_wait(17, 1, 0), 1)
            close.assert_called_once_with(17)
            pause.assert_not_called()


PASS = "test result: ok. 1 passed; 0 failed; 0 ignored; 2 filtered out\n"
ZERO = "test result: ok. 0 passed; 0 failed; 0 ignored; 3 filtered out\n"


@unittest.skipUnless(launcher.process_support_error() is None,
                     "owned process groups require POSIX pipes and waitpid")
class RealProcessTests(unittest.TestCase):
    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.output = io.StringIO()
        self.stack.enter_context(contextlib.redirect_stdout(self.output))
        self.stack.enter_context(contextlib.redirect_stderr(self.output))
        self.directory = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="watch-scenarios-")))
        self.stack.enter_context(patch.object(launcher, "BUILD_SECONDS", 2))
        self.stack.enter_context(patch.object(launcher, "EXECUTION_SECONDS", 2))
        self.stack.enter_context(patch.object(launcher, "TERM_GRACE_SECONDS", 0.1))
        self.stack.enter_context(patch.object(launcher, "KILL_GRACE_SECONDS", 0.5))

    def cargo(self, body):
        path = self.directory / "cargo stand-in"
        path.write_text(f"#!{sys.executable}\n" + textwrap.dedent(body))
        path.chmod(0o700)
        return str(path)

    def run_cargo(self, body):
        return launcher.run_scenario(launcher.SCENARIOS[0], self.cargo(body))

    def assert_stopped(self, pid):
        # A killed orphan can remain a zombie until the container's init reaps it.
        # It is no longer runnable and cannot keep the original work alive.
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            stat = Path(f"/proc/{pid}/stat")
            if stat.exists():
                try:
                    if stat.read_text().rsplit(")", 1)[1].split()[0] == "Z":
                        return
                except FileNotFoundError:
                    continue  # Recheck the PID after a procfs disappearance race.
            if not Path("/proc").exists():
                # Native POSIX control for systems without Linux procfs. Missing
                # ps is an unsupported assertion, not evidence of child exit.
                status = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)],
                                        capture_output=True, text=True, timeout=2)
                if status.returncode == 0 and status.stdout.lstrip().startswith("Z"):
                    return
            time.sleep(0.02)
        self.fail(f"owned process {pid} is still alive")

    def test_missing_native_observer_cannot_report_a_live_child_stopped(self):
        with patch.object(os, "kill", return_value=None), \
             patch.object(Path, "exists", return_value=False), \
             patch.object(subprocess, "run", side_effect=FileNotFoundError("ps unavailable")):
            with self.assertRaisesRegex(FileNotFoundError, "ps unavailable"):
                self.assert_stopped(123456)

    def test_real_two_phases_keep_cwd_and_caller_environment(self):
        records = self.directory / "records.jsonl"
        values = {"WATCH_LAUNCHER_CHECK": "vimes", "TZ": "Pacific/Honolulu",
                  "RADAR_SANDBOX": "caller-choice", "LD_LIBRARY_PATH": "/watch/library",
                  "DYLD_FALLBACK_LIBRARY_PATH": "/watch/fallback", "CARGO_TARGET_DIR": "/watch/target",
                  "PATH": os.environ.get("PATH", ""), "PYTHONPATH": "/watch/python"}
        with patch.dict(os.environ, values):
            self.assertEqual(self.run_cargo(f"""
                import json, os, sys
                with open({str(records)!r}, "a") as log:
                    log.write(json.dumps({{"args": sys.argv[1:], "cwd": os.getcwd(),
                        "env": {{key: os.environ.get(key) for key in {list(values)!r}}},
                        "pid": os.getpid(), "ppid": os.getppid(), "pgid": os.getpgrp(), "sid": os.getsid(0)}}) + "\\n")
                if "--no-run" not in sys.argv:
                    print({PASS!r}, end="")
                """), 0)
        entries = [json.loads(line) for line in records.read_text().splitlines()]
        self.assertEqual(len(entries), 2)
        self.assertEqual(entries[0]["args"], launcher.build_command(launcher.SCENARIOS[0])[1:])
        self.assertEqual(entries[1]["args"], launcher.command(launcher.SCENARIOS[0])[1:])
        for entry in entries:
            self.assertEqual(entry["cwd"], str(launcher.ROOT))
            self.assertEqual(entry["env"], values)
            self.assertNotEqual(entry["pid"], entry["pgid"])
            self.assertEqual(entry["pgid"], entry["sid"])
            self.assertEqual(entry["ppid"], entry["pgid"])
            self.assertNotEqual(entry["pgid"], os.getpgrp())
        self.assertNotEqual(entries[0]["pgid"], entries[1]["pgid"])

    def test_real_build_error_does_not_start_execution(self):
        marker = self.directory / "execution"
        self.assertEqual(self.run_cargo(f"""
            import pathlib, sys
            if "--no-run" in sys.argv:
                print("compile failed", file=sys.stderr)
                sys.exit(101)
            pathlib.Path({str(marker)!r}).touch()
            """), 101)
        self.assertFalse(marker.exists())
        self.assertIn("compile failed", self.output.getvalue())

    def test_real_execution_error_and_zero_ignored_or_stale_output_fail(self):
        for code, output, expected in [(73, PASS, 73), (0, ZERO, 1),
                (0, "test result: ok. 0 passed; 0 failed; 1 ignored;\n", 1), (0, "", 1)]:
            with self.subTest(code=code, output=output):
                self.assertEqual(self.run_cargo(f"""
                    import sys
                    if "--no-run" in sys.argv:
                        print({PASS!r}, end="")
                    else:
                        print({output!r}, end="")
                        sys.exit({code})
                    """), expected)

    def test_real_diagnostic_memory_and_display_are_bounded_per_phase(self):
        for phase in ("build", "execution"):
            with self.subTest(phase=phase):
                self.output.seek(0)
                self.output.truncate()
                self.assertEqual(self.run_cargo(f"""
                    import sys
                    if ("--no-run" in sys.argv) == ({phase!r} == "build"):
                        sys.stdout.write("discarded-prefix" + "x" * {launcher.DIAGNOSTIC_BYTES * 4}
                                         + "terminal failure\\n")
                        sys.exit(101)
                    """), 101)
                output = self.output.getvalue()
                self.assertNotIn("discarded-prefix", output)
                self.assertIn("terminal failure", output)
                self.assertIn("Showing last", output)
                self.assertLess(len(output), launcher.DIAGNOSTIC_BYTES + 2000)

    def test_both_deadlines_include_startup_and_revalidation(self):
        for phase in ("build", "execution"):
            with self.subTest(phase=phase), \
                 patch.object(launcher, "BUILD_SECONDS", 0.15 if phase == "build" else 2), \
                 patch.object(launcher, "EXECUTION_SECONDS", 0.15):
                self.output.seek(0)
                self.output.truncate()
                marker = self.directory / (phase + "-execution")
                started = time.monotonic()
                self.assertEqual(self.run_cargo(f"""
                    import pathlib, sys, time
                    build = "--no-run" in sys.argv
                    if not build:
                        pathlib.Path({str(marker)!r}).touch()
                    if build == ({phase!r} == "build"):
                        print("waiting for Cargo lock / revalidation", flush=True)
                        print({PASS!r}, end="", flush=True)
                        time.sleep(30)
                    """), 124)
                self.assertLess(time.monotonic() - started, 2)
                self.assertEqual(marker.exists(), phase == "execution")
                self.assertIn(phase.title() + " timed out after 0.15s", self.output.getvalue())
                if phase == "execution":
                    self.assertIn("includes Cargo startup and revalidation", self.output.getvalue())

    def test_signal_handlers_are_restored_on_success_failure_and_missing_cargo(self):
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        for argv in ([sys.executable, "-c", "pass"],
                     [sys.executable, "-c", "raise SystemExit(17)"],
                     [str(self.directory / "missing-cargo")]):
            with self.subTest(argv=argv):
                try:
                    launcher.run_phase(argv, 2)
                except FileNotFoundError:
                    pass
                self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)

    def tree_body(self, phase, interrupt=0, exit_on_term=False, exit_early=False, kill_helper=False):
        # All descendants stay in Cargo's owned group. The grandchild ignores
        # TERM so the test proves KILL still reaches it after its parent exits.
        descendant = textwrap.dedent(f"""
            import os, pathlib, signal, subprocess, sys, time
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            subprocess.Popen([sys.executable, "-c",
                "import os, pathlib, signal, time; "
                "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                "pathlib.Path({str(self.directory / 'grandchild')!r}).write_text(str(os.getpid())); "
                "time.sleep(30)"])
            pathlib.Path({str(self.directory / 'child')!r}).write_text(str(os.getpid()))
            time.sleep(30)
            """)
        return f"""
            import os, pathlib, signal, subprocess, sys, time
            if ("--no-run" in sys.argv) != ({phase!r} == "build"):
                print({PASS!r}, end="")
                sys.exit(0)
            if not {exit_on_term!r}:
                signal.signal(signal.SIGTERM, signal.SIG_IGN)
            subprocess.Popen([sys.executable, "-c", {descendant!r}])
            pathlib.Path({str(self.directory / 'leader')!r}).write_text(str(os.getpid()))
            ready = pathlib.Path({str(self.directory / 'grandchild')!r})
            deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            if {kill_helper!r}:
                os.kill(os.getppid(), signal.SIGKILL)
            if {interrupt}:
                os.kill(int(pathlib.Path({str(self.directory / "wrapper")!r}).read_text()), {interrupt})
            if {exit_early!r}:
                print({PASS!r}, end="", flush=True)
                sys.exit(0)
            time.sleep(30)
            """

    def launch_isolated_wrapper(self, cargo, phase, timeout, setup="", input_text=None):
        # Execute the launcher in a separate process so real SIGINT/SIGTERM do
        # not target unittest. This watchdog is test-only and never runs Cargo.
        source = f"""
import importlib.machinery, importlib.util, os, pathlib, sys
sys.dont_write_bytecode = True
pathlib.Path({str(self.directory / "wrapper")!r}).write_text(str(os.getpid()))
loader = importlib.machinery.SourceFileLoader("watch_launcher", {str(path)!r})
spec = importlib.util.spec_from_loader(loader.name, loader)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
loader.exec_module(module)
module.BUILD_SECONDS = {timeout if phase == 'build' else 2}
module.EXECUTION_SECONDS = {timeout if phase == 'execution' else 2}
module.TERM_GRACE_SECONDS = 0.1
module.KILL_GRACE_SECONDS = 0.5
{setup}
raise SystemExit(module.main(["slack-success", "--cargo", {cargo!r}]))
"""
        return subprocess.run([sys.executable, "-c", source], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=8, input=input_text)

    def test_timeout_and_interrupt_kill_owned_tree_only(self):
        sentinel = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                    start_new_session=True)
        try:
            for phase in ("build", "execution"):
                for interrupt in (0, signal.SIGINT, signal.SIGTERM):
                    for exit_on_term in (False, True):
                        with self.subTest(phase=phase, interrupt=interrupt, exit_on_term=exit_on_term):
                            for name in ("leader", "child", "grandchild"):
                                (self.directory / name).unlink(missing_ok=True)
                            cargo = self.cargo(self.tree_body(phase, interrupt, exit_on_term))
                            started = time.monotonic()
                            result = self.launch_isolated_wrapper(cargo, phase, 0.5 if not interrupt else 2)
                            self.assertEqual(result.returncode, 128 + interrupt if interrupt else 124,
                                             result.stdout)
                            self.assertLess(time.monotonic() - started, 4)
                            for name in ("leader", "child", "grandchild"):
                                self.assert_stopped(int((self.directory / name).read_text()))
                            self.assertIsNone(sentinel.poll())
                            self.assertIn(phase.title() + (" interrupted" if interrupt else " timed out"),
                                          result.stdout)
        finally:
            sentinel.terminate()
            sentinel.wait(timeout=2)

    def test_successful_leader_exit_cleans_lingering_descendants_before_next_phase(self):
        for phase in ("build", "execution"):
            with self.subTest(phase=phase):
                for name in ("leader", "child", "grandchild"):
                    (self.directory / name).unlink(missing_ok=True)
                cargo = self.cargo(self.tree_body(phase, exit_early=True))
                result = self.launch_isolated_wrapper(cargo, phase, 2)
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertIn("Execution (deadline", result.stdout)
                for name in ("leader", "child", "grandchild"):
                    self.assert_stopped(int((self.directory / name).read_text()))

    def helper_script(self, body):
        script = self.directory / "helper.py"
        script.write_text(textwrap.dedent(body))
        return script

    def test_real_launch_works_without_waitid_or_wnowait(self):
        cargo = self.cargo(f"print({PASS!r}, end='')")
        setup = "\n".join([
            "for name in ('waitid', 'WNOWAIT', 'P_PID', 'WEXITED'):",
            "    if hasattr(os, name): delattr(os, name)",
        ])
        result = self.launch_isolated_wrapper(cargo, "execution", 2, setup=setup)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_real_stdin_is_inherited_through_supervisor(self):
        cargo = self.cargo(f"""
            import sys
            if '--no-run' not in sys.argv:
                assert sys.stdin.read() == 'watch input'
                print({PASS!r}, end='')
            """)
        result = self.launch_isolated_wrapper(cargo, "execution", 2, input_text="watch input")
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_early_dead_stopped_or_silent_helper_fails_and_is_reaped(self):
        pid_file = self.directory / "supervisor"
        for mode in ("dead", "stopped", "silent"):
            with self.subTest(mode=mode):
                script = self.helper_script(f"""
                    import os, pathlib, signal, time
                    pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()))
                    if {mode!r} == 'dead':
                        os.kill(os.getpid(), signal.SIGKILL)
                    if {mode!r} == 'stopped':
                        os.kill(os.getpid(), signal.SIGSTOP)
                    time.sleep(30)
                    """)
                with patch.object(launcher, "SUPERVISOR", script):
                    result = launcher.run_phase(["never-start-cargo"], 0.2)
                self.assertFalse(result.cleanup_failed)
                self.assertTrue(result.error if mode == "dead" else result.timed_out)
                pid = int(pid_file.read_text())
                with self.assertRaises(ChildProcessError):
                    os.waitpid(pid, os.WNOHANG)

    def test_helper_death_while_cargo_tree_runs_still_cleans_the_reserved_group(self):
        sentinel = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], start_new_session=True)
        try:
            for phase in ("build", "execution"):
                with self.subTest(phase=phase):
                    for name in ("leader", "child", "grandchild"):
                        (self.directory / name).unlink(missing_ok=True)
                    cargo = self.cargo(self.tree_body(phase, kill_helper=True))
                    result = self.launch_isolated_wrapper(cargo, phase, 2)
                    self.assertEqual(result.returncode, 1, result.stdout)
                    self.assertIn("status was empty", result.stdout)
                    for name in ("leader", "child", "grandchild"):
                        self.assert_stopped(int((self.directory / name).read_text()))
                    self.assertIsNone(sentinel.poll())
        finally:
            sentinel.terminate()
            sentinel.wait(timeout=2)

    def test_helper_death_after_real_cargo_report_preserves_cargo_result(self):
        pid_file = self.directory / "supervisor"
        script = self.helper_script(f"""
            import os, pathlib, runpy, signal
            pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()))
            signal.pause = lambda: os.kill(os.getpid(), signal.SIGKILL)
            runpy.run_path({str(launcher.SUPERVISOR)!r}, run_name='__main__')
            """)
        with patch.object(launcher, "SUPERVISOR", script):
            result = launcher.run_phase([sys.executable, "-c", "raise SystemExit(73)"], 2)
        self.assertEqual(result.returncode, 73)
        self.assertFalse(result.cleanup_failed)
        self.assertFalse(result.error)
        with self.assertRaises(ChildProcessError):
            os.waitpid(int(pid_file.read_text()), os.WNOHANG)

    def test_real_status_rejects_bad_reports_stdout_spoofs_and_missing_eof(self):
        valid = launcher.REPORT.pack(launcher.REPORT_MAGIC, 1, 0)
        for record in (b"", valid[:3], b"BAD!" + valid[4:], valid * 2, valid):
            with self.subTest(record=record):
                script = self.helper_script(f"""
                    import os, sys, time
                    print({PASS!r}, end='', flush=True)
                    print({valid!r}, flush=True)
                    os.write(int(sys.argv[1]), {record!r})
                    if {record != valid!r}:
                        os.close(int(sys.argv[1]))
                    time.sleep(30)
                    """)
                with patch.object(launcher, "SUPERVISOR", script):
                    result = launcher.run_phase(["never-start-cargo"], 0.2)
                self.assertFalse(result.cleanup_failed)
                self.assertTrue(result.timed_out if record == valid else result.error)
                self.assertIn("1 passed", result.output)

    def test_private_status_descriptor_is_absent_in_cargo_and_grandchild(self):
        identity = self.directory / "pipe-identity.json"
        script = self.helper_script(f"""
            import json, os, pathlib, runpy, sys
            status = os.fstat(int(sys.argv[1]))
            pathlib.Path({str(identity)!r}).write_text(json.dumps([status.st_dev, status.st_ino]))
            runpy.run_path({str(launcher.SUPERVISOR)!r}, run_name='__main__')
            """)
        check = textwrap.dedent(f"""
            import json, os, pathlib
            identity = json.loads(pathlib.Path({str(identity)!r}).read_text())
            for fd in range(256):
                try:
                    status = os.fstat(fd)
                except OSError:
                    continue
                assert [status.st_dev, status.st_ino] != identity, (fd, identity)
            """)
        cargo = self.cargo(f"""
            import subprocess, sys
            exec({check!r})
            subprocess.run([sys.executable, '-c', {check!r}], check=True)
            print({PASS!r}, end='')
            """)
        with patch.object(launcher, "SUPERVISOR", script):
            self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0], cargo), 0)

    @unittest.skipUnless(Path('/proc/self/fd').exists(), 'Linux descriptor-count control; native platforms need their own run')
    def test_repeated_success_and_failure_do_not_leak_descriptors(self):
        before = set(os.listdir('/proc/self/fd'))
        for code in (0, 73, 0, 73):
            result = launcher.run_phase([sys.executable, '-c', f'raise SystemExit({code})'], 2)
            self.assertEqual(result.returncode, code)
            self.assertFalse(result.cleanup_failed)
        self.assertEqual(set(os.listdir('/proc/self/fd')), before)


if __name__ == "__main__":
    unittest.main()
