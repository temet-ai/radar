#!/usr/bin/env python3
"""Small regression suite for the offline documentation checker."""

import runpy
import tempfile
import unittest
from pathlib import Path

CHECKER = runpy.run_path(str(Path(__file__).with_name("check-docs")))


class DocumentationChecks(unittest.TestCase):
    def test_heading_punctuation_code_and_duplicates(self):
        self.assertEqual(CHECKER["anchors"]("# Install `radar`!\n## Install radar\n## Install radar\n"),
                         {"install-radar", "install-radar-1", "install-radar-2"})

    def test_fenced_examples_and_inline_code_are_ignored(self):
        text = "```md\n[example](missing.md)\n```\n`[code](missing.md)`\n[real](page.md#start)\n"
        self.assertEqual(list(CHECKER["destinations"](text)), [(5, "page.md#start")])

    def test_images_reference_links_and_html(self):
        text = '![shot](image.png)\n[guide]: <page.md#start>\n<img src="other.png">\n'
        self.assertEqual([d for _, d in CHECKER["destinations"](text)],
                         ["image.png", "page.md#start", "other.png"])

    def test_missing_file_and_anchor_fail_but_external_is_not_fetched(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "README.md"
            source.write_text("# Start\n[ok](#start)\n[bad](#absent)\n[missing](gone.md)\n[web](https://example.com)\n")
            checked, failures = CHECKER["check"]([source], root)
            self.assertEqual(checked, 3)
            self.assertEqual(len(failures), 2)
            self.assertIn("missing heading", failures[0])
            self.assertIn("missing target", failures[1])


if __name__ == "__main__":
    unittest.main()
