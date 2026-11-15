"""Docs sitemap must not claim every page changed at deploy time."""

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import xml.etree.ElementTree as ET

from scripts.docs.sitemap_lastmod import on_post_build


SITEMAP = """<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
    <url>
        <loc>https://docs.labwired.com/</loc>
        <lastmod>2026-11-15</lastmod>
    </url>
    <url>
        <loc>https://docs.labwired.com/agents/</loc>
        <lastmod>2026-11-15</lastmod>
    </url>
</urlset>
"""


class SitemapFreshnessTests(unittest.TestCase):
    def test_preserves_urls_and_removes_unverified_lastmod(self):
        with TemporaryDirectory() as tmp:
            path = Path(tmp) / "sitemap.xml"
            path.write_text(SITEMAP, encoding="utf-8")
            on_post_build({"site_dir": tmp})
            root = ET.parse(path).getroot()
            ns = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}
            locs = [x.text for x in root.findall("sm:url/sm:loc", ns)]
            self.assertEqual(locs, ["https://docs.labwired.com/", "https://docs.labwired.com/agents/"])
            self.assertEqual(root.findall("sm:url/sm:lastmod", ns), [])

    def test_is_idempotent(self):
        with TemporaryDirectory() as tmp:
            path = Path(tmp) / "sitemap.xml"
            path.write_text(SITEMAP, encoding="utf-8")
            on_post_build({"site_dir": tmp})
            after = path.read_bytes()
            on_post_build({"site_dir": tmp})
            self.assertEqual(path.read_bytes(), after)

    def test_missing_sitemap_is_error(self):
        with TemporaryDirectory() as tmp:
            with self.assertRaises(FileNotFoundError):
                on_post_build({"site_dir": tmp})


if __name__ == "__main__":
    unittest.main()
