#!/usr/bin/env python3
"""No-build checks for the headless launcher, using only the Python stdlib."""

import contextlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
path = Path(__file__).with_name("scenarios")
loader = importlib.machinery.SourceFileLoader("scenario_launcher", str(path))
spec = importlib.util.spec_from_loader(loader.name, loader)
launcher = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = launcher
loader.exec_module(launcher)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.output = io.StringIO()
        self.enterContext(contextlib.redirect_stdout(self.output))
        self.enterContext(contextlib.redirect_stderr(self.output))

    def test_list_never_runs_cargo(self):
        with patch.object(launcher.subprocess, "run") as run:
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
            with patch.object(launcher.subprocess, "run") as run:
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
        with patch.object(launcher.subprocess, "run") as run:
            for args in cases:
                with self.subTest(args=args), self.assertRaises(SystemExit) as raised:
                    launcher.main(args)
                self.assertEqual(raised.exception.code, 2)
        run.assert_not_called()

    def test_dry_run_selects_without_build(self):
        with patch.object(launcher.subprocess, "run") as run:
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

    def fake_run(self, output, code=0):
        def execute(argv, **kwargs):
            self.assertEqual(kwargs["cwd"], launcher.ROOT)
            self.assertNotIn("env", kwargs)  # caller's Rust/Cargo environment is inherited
            self.assertNotIn("shell", kwargs)
            kwargs["stdout"].write(output)
            return subprocess.CompletedProcess(argv, code)
        return execute

    def test_single_pass_is_required(self):
        for output, expected in [
            (b"test result: ok. 1 passed; 0 failed; 0 ignored; 2 filtered out\n", 0),
            (b"test result: ok. 0 passed; 0 failed; 0 ignored; 3 filtered out\n", 1),
            (b"test result: ok. 0 passed; 0 failed; 1 ignored; 2 filtered out\n", 1),
        ]:
            with patch.object(launcher.subprocess, "run", side_effect=self.fake_run(output)):
                self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), expected)

    def test_failure_diagnostics_are_bounded(self):
        output = b"discarded-prefix" + b"x" * launcher.DIAGNOSTIC_BYTES + b"compile failed\n"
        with patch.object(launcher.subprocess, "run", side_effect=self.fake_run(output, 101)):
            self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), 101)
        self.assertNotIn("discarded-prefix", self.output.getvalue())
        self.assertIn("compile failed", self.output.getvalue())
        self.assertIn("Showing last", self.output.getvalue())

    def test_missing_cargo_is_actionable(self):
        with patch.object(launcher.subprocess, "run", side_effect=FileNotFoundError("cargo")):
            self.assertEqual(launcher.run_scenario(launcher.SCENARIOS[0]), 1)
        self.assertIn("--cargo /path/to/cargo", self.output.getvalue())


if __name__ == "__main__":
    unittest.main()
