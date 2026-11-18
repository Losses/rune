"""Check the cover-art CLI example against its documented boolean semantics.

Run with: python3 -m unittest discover -s scripts -p test_mix_query_documentation.py
"""

from pathlib import Path
import re
import unittest


class MixQueryDocumentationTest(unittest.TestCase):
    def test_cover_art_example_keeps_tracks_with_cover_art(self):
        document = (
            Path(__file__).resolve().parents[1] / "documents/mix_query_syntax.md"
        ).read_text(encoding="utf-8")
        example = document.split("1. **Filter and Sort Media Files**", 1)[1].split(
            "2. **Generate Recommendations**", 1
        )[0]
        command = re.search(r"```bash\s*(.*?)\s*```", example, re.DOTALL)
        self.assertIsNotNone(command, "The cover-art example must include a command")
        filters = re.findall(r"filter::with_cover_art\(([^)]+)\)", command[1])
        self.assertEqual(filters, ["true"], "The example must keep tracks with cover art")
        self.assertIn("filters out those without cover art", example)
        table_row = next(
            line for line in document.splitlines()
            if "**filter::with_cover_art**" in line
        )
        self.assertIn("`true` for with cover arts", table_row)
        self.assertIn("`false` for without cover arts", table_row)


if __name__ == "__main__":
    unittest.main()
