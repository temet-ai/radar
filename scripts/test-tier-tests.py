#!/usr/bin/env python3
"""Policy checker regression tests; stdlib only, no Rust build or external I/O."""
import copy
import importlib.machinery
import importlib.util
from pathlib import Path
import unittest
import sys

sys.dont_write_bytecode = True

loader = importlib.machinery.SourceFileLoader("test_tiers", str(Path(__file__).with_name("check-test-tiers")))
spec = importlib.util.spec_from_loader(loader.name, loader)
checker = importlib.util.module_from_spec(spec)
loader.exec_module(checker)


def policy():
    return {"version": 1, "scopes": [{"id": "models", "package": "radar-watch",
            "binary": "radar_watch", "prefix": "fixture::tests::", "cases": [
                {"name": "fixture::tests::owns_values", "tier": "unit", "parts": ["watch"],
                 "boundary": "Owned in-memory model defaults"}]}]}


def inventory():
    return {("radar-watch", "radar_watch"): {"fixture::tests::owns_values": False,
                                             "legacy::mixed": False}}


def nextest():
    return {"rust-suites": {"radar-watch": {"package-name": "radar-watch",
            "binary-name": "radar_watch", "status": "listed", "testcases": {
                "fixture::tests::owns_values": {"ignored": False,
                                                "filter-match": {"status": "matches"}}}}}}


class PolicyTests(unittest.TestCase):
    def test_unknown_backlog_is_visible_and_never_unit(self):
        errors, classified, backlog = checker.check(checker.validate_policy(policy()), inventory())
        self.assertEqual(errors, [])
        self.assertEqual(list(classified.values()), ["unit"])
        self.assertEqual(backlog, [("radar-watch", "radar_watch", "legacy::mixed")])

    def test_strict_whole_inventory_rejects_backlog(self):
        errors, _, _ = checker.check(policy()["scopes"], inventory(), reject_backlog=True)
        self.assertIn("unclassified tests remain", errors[0])

    def test_new_case_inside_migrated_module_is_rejected(self):
        found = inventory()
        found[("radar-watch", "radar_watch")]["fixture::tests::new_case"] = False
        errors, _, _ = checker.check(policy()["scopes"], found)
        self.assertIn("unclassified addition", errors[0])

    def test_missing_declared_case_is_rejected(self):
        errors, _, _ = checker.check(policy()["scopes"], {("radar-watch", "radar_watch"): {}})
        self.assertIn("missing declared test", errors[0])

    def test_missing_feature_or_binary_is_not_green(self):
        errors, _, _ = checker.check(policy()["scopes"], {})
        self.assertIn("missing binary", errors[0])

    def test_ignored_regression_is_rejected(self):
        found = inventory()
        found[("radar-watch", "radar_watch")]["fixture::tests::owns_values"] = True
        errors, _, _ = checker.check(policy()["scopes"], found)
        self.assertIn("ignored state changed", errors[0])

    def test_partial_scope_must_be_known(self):
        with self.assertRaisesRegex(ValueError, "unknown scopes"):
            checker.check(policy()["scopes"], inventory(), ["typo"])

    def test_scopes_cannot_overlap(self):
        data = policy()
        other = copy.deepcopy(data["scopes"][0])
        other.update(id="everything", prefix="")
        data["scopes"].append(other)
        with self.assertRaisesRegex(ValueError, "overlapping"):
            checker.validate_policy(data)

    def test_module_prefix_cannot_accidentally_match_another_module(self):
        data = policy()
        data["scopes"][0]["prefix"] = "fixture"
        with self.assertRaisesRegex(ValueError, "module boundary"):
            checker.validate_policy(data)

    def test_duplicate_test_declaration_rejected(self):
        data = policy()
        data["scopes"][0]["cases"] *= 2
        with self.assertRaisesRegex(ValueError, "duplicate case"):
            checker.validate_policy(data)

    def test_non_unit_requires_lower_tier_justification(self):
        data = policy()
        data["scopes"][0]["cases"][0]["tier"] = "contract"
        with self.assertRaisesRegex(ValueError, "lower-tier justification"):
            checker.validate_policy(data)

    def test_every_case_requires_boundary_and_parts(self):
        for field, expected in [("boundary", "protected boundary"), ("parts", "minimal parts")]:
            with self.subTest(field=field):
                data = policy()
                del data["scopes"][0]["cases"][0][field]
                with self.assertRaisesRegex(ValueError, expected):
                    checker.validate_policy(data)

    def test_invalid_tier_is_rejected(self):
        data = policy()
        data["scopes"][0]["cases"][0]["tier"] = "fast-enough"
        with self.assertRaisesRegex(ValueError, "invalid tier"):
            checker.validate_policy(data)

    def test_nextest_inventory(self):
        self.assertEqual(checker.read_nextest(nextest()), {
            ("radar-watch", "radar_watch"): {"fixture::tests::owns_values": False}})

    def test_nextest_filtered_suite_rejected(self):
        data = nextest()
        data["rust-suites"]["radar-watch"]["status"] = "skipped"
        with self.assertRaisesRegex(ValueError, "not listed"):
            checker.read_nextest(data)

    def test_nextest_filtered_case_rejected(self):
        data = nextest()
        data["rust-suites"]["radar-watch"]["testcases"]["fixture::tests::owns_values"]["filter-match"]["status"] = "mismatch"
        with self.assertRaisesRegex(ValueError, "filtered test"):
            checker.read_nextest(data)

    def test_default_ignored_case_is_discovered(self):
        data = nextest()
        data["rust-suites"]["radar-watch"]["testcases"]["legacy::live"] = {
            "ignored": True, "filter-match": {"status": "mismatch", "reason": "ignored"}}
        self.assertTrue(checker.read_nextest(data)[("radar-watch", "radar_watch")]["legacy::live"])

    def test_binaries_only_is_not_test_discovery(self):
        with self.assertRaisesRegex(ValueError, "not binaries-only"):
            checker.read_nextest({"rust-binaries": {}})

    def test_empty_inventory_is_not_green(self):
        with self.assertRaisesRegex(ValueError, "empty discovered"):
            checker.read_nextest({"rust-suites": {}})
        with self.assertRaisesRegex(ValueError, "empty libtest"):
            checker.read_libtest("")

    def test_duplicate_binary_is_ambiguous(self):
        data = nextest()
        data["rust-suites"]["duplicate"] = copy.deepcopy(data["rust-suites"]["radar-watch"])
        with self.assertRaisesRegex(ValueError, "duplicate discovered binary"):
            checker.read_nextest(data)

    def test_empty_lib_and_bin_with_same_name_are_allowed(self):
        data = nextest()
        data["rust-suites"]["radar-watch"]["testcases"] = {}
        data["rust-suites"]["bin"] = copy.deepcopy(data["rust-suites"]["radar-watch"])
        self.assertEqual(checker.read_nextest(data), {("radar-watch", "radar_watch"): {}})

    def test_libtest_listing_does_not_invent_ignored_state(self):
        self.assertEqual(checker.read_libtest("fixture::tests::owns_values: test\n"),
                         {"fixture::tests::owns_values": None})

    def test_libtest_bad_lines_and_duplicates_fail(self):
        for content in ["running 1 test", "a: test\na: test", "a: benchmark"]:
            with self.subTest(content=content), self.assertRaises(ValueError):
                checker.read_libtest(content)


if __name__ == "__main__":
    unittest.main()
