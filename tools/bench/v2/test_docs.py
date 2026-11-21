"""The documentation pages are made from the published results and the case files, so they must match them.

If this fails, a result or a case changed without the pages being rewritten: run `make docs` and commit the pages.
"""
import unittest
from pathlib import Path

import aggregate
import cases_page
import pages
import publish
import scoring

HERE = Path(__file__).resolve().parent
DOCS = HERE.parents[2] / "website/src/content/docs/docs/benchmarks"


class DocsAreCurrent(unittest.TestCase):
    def test_the_results_pages_match_the_published_results(self):
        results, profiles, _ = aggregate.load_published(publish.STORE, publish.current_suite())
        for slug, text in pages.pages(results, profiles, None, stubs=True).items():
            self.assertEqual((DOCS / f"{slug}.md").read_text(), text, f"{slug}.md is out of date: run `make docs`")

    def test_the_cases_page_matches_the_cases(self):
        text = cases_page.build(scoring.load_cases(HERE / "cases", HERE / "fixtures"))
        self.assertEqual((DOCS / "cases-v2.md").read_text(), text, "cases-v2.md is out of date: run `make docs`")

    def test_the_front_matter_of_every_page_is_valid_yaml_text(self):
        import json
        for path in sorted(DOCS.glob("*-v2.md")):
            lines = path.read_text().split("\n")
            self.assertEqual(lines[0], "---", path.name)
            end = lines.index("---", 1)
            for line in lines[1:end]:
                key, _, value = line.partition(": ")
                # a value with a colon, quote or hash must be a quoted string, or YAML reads it wrongly
                if any(ch in value for ch in ":#\"'"):
                    self.assertEqual(json.loads(value), json.loads(value), f"{path.name}: {line}")
                    self.assertTrue(value.startswith('"') and value.endswith('"'), f"{path.name}: unquoted {key}")

    def test_the_sidebar_lists_every_generated_page(self):
        config = (HERE.parents[2] / "website/astro.config.mjs").read_text()
        for slug in list(pages.PAGE_SLUG.values()) + ["cases-v2", "evaluation-v2"]:
            self.assertIn(f"docs/benchmarks/{slug}'", config, f"{slug} is not in the sidebar")


if __name__ == "__main__":
    unittest.main()
