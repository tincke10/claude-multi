"""Tests for the README header generator. Run: python3 -m unittest discover -s docs/readme"""
import re
import tempfile
import unittest
from pathlib import Path

import render

REPO = Path(__file__).resolve().parents[2]
README = (REPO / "README.md").read_text(encoding="utf-8")
FONT = "AAAA"  # tiny stand-in; the font payload is irrelevant to these tests


def gh_slug(heading: str) -> str:
    s = re.sub(r"[^\w\- ]", "", heading.strip().lower())
    return s.replace(" ", "-")


class EscapeTest(unittest.TestCase):
    def test_should_escape_markup_when_text_has_special_chars(self):
        self.assertEqual(render.esc('a<b>&"c"'), "a&lt;b&gt;&amp;&quot;c&quot;")

    def test_should_not_leak_raw_markup_when_pill_label_has_angle_brackets(self):
        svg = render.pill_svg("<x>", FONT)
        self.assertNotIn("<x>", svg)
        self.assertIn("&lt;x&gt;", svg)


class DeterminismTest(unittest.TestCase):
    def test_should_render_identical_bytes_when_rendered_twice(self):
        a = render.session_svg("9.9.9", FONT)
        b = render.session_svg("9.9.9", FONT)
        self.assertEqual(a.encode(), b.encode())
        self.assertEqual(render.pill_svg("install", FONT).encode(), render.pill_svg("install", FONT).encode())

    def test_should_keep_every_session_line_inside_the_viewport(self):
        for row in render.session_rows("0.0.0"):
            if isinstance(row, list):
                self.assertLessEqual(sum(len(t) for t, _ in row), render.COLS, row)


class VersionTest(unittest.TestCase):
    def test_should_read_version_from_the_package_section(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "Cargo.toml"
            p.write_text('[package]\nname = "x"\nversion = "1.2.3"\n\n[dependencies]\nfoo = { version = "9" }\n')
            self.assertEqual(render.read_version(p), "1.2.3")

    def test_should_match_the_real_cargo_toml(self):
        cargo = (REPO / "src-tauri" / "Cargo.toml").read_text()
        expected = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)
        self.assertEqual(render.read_version(REPO / "src-tauri" / "Cargo.toml"), expected)

    def test_should_show_the_version_in_the_title_bar_and_in_cms_output(self):
        svg = render.session_svg("4.5.6", FONT)
        self.assertIn("v4.5.6", svg)
        self.assertIn("cms 4.5.6", svg)


class PillsTest(unittest.TestCase):
    def test_should_have_a_pill_file_for_every_pill_the_readme_references(self):
        refs = re.findall(r'src="(docs/readme/pills/[^"]+\.svg)"', README)
        self.assertTrue(refs, "README references no pills")
        for ref in refs:
            self.assertTrue((REPO / ref).is_file(), f"missing {ref}")

    def test_should_reference_the_session_image_from_the_readme(self):
        self.assertIn('src="docs/readme/session.svg"', README)
        self.assertTrue((REPO / "docs/readme/session.svg").is_file())

    def test_should_link_every_pill_to_an_existing_anchor_or_file(self):
        headings = {gh_slug(h) for h in re.findall(r"^#{1,6} (.+)$", README, re.M)}
        for _, href in render.PILLS:
            if href.startswith("#"):
                self.assertIn(href[1:], headings, href)
            else:
                self.assertTrue((REPO / href).exists(), href)

    def test_should_use_the_same_pill_links_in_the_readme_as_the_generator(self):
        for label, href in render.PILLS:
            self.assertIn(f'href="{href}"', README, label)
            self.assertIn(f"pills/{render.slug(label)}.svg", README, label)


if __name__ == "__main__":
    unittest.main()
