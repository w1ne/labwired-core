"""Public docs link to repository-only evidence by immutable source revision."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
PAGES = (
    ROOT / 'docs/index.md',
    ROOT / 'docs/boards/luckfox-pico-mini-a.md',
    ROOT / 'docs/qemu-arm-core.md',
)
SOURCE_LINK = re.compile(
    r'\]\(https://github\.com/w1ne/labwired-core/(blob|tree)/([0-9a-f]{40})/'
    r'((?:examples|configs|\.github)/[^)]+)\)'
)
BAD_RELATIVE = re.compile(r'\]\((?:\.\./)+(?:examples|configs|\.github)/')


class RepositoryEvidenceLinks(unittest.TestCase):
    def test_published_pages_do_not_link_out_of_the_docs_site(self):
        for page in PAGES:
            with self.subTest(page=page):
                self.assertNotRegex(page.read_text(), BAD_RELATIVE)

    def test_source_evidence_uses_pinned_links_to_real_files(self):
        counts = 0
        for page in PAGES:
            for kind, revision, path in SOURCE_LINK.findall(page.read_text()):
                with self.subTest(page=page, path=path):
                    target = ROOT / path
                    self.assertTrue(target.exists(), path)
                    self.assertEqual(kind, 'tree' if target.is_dir() else 'blob')
                    self.assertEqual(len(revision), 40)
                    counts += 1
        self.assertEqual(counts, 19)

    def test_homepage_has_a_source_link_for_the_linux_basket(self):
        text = (ROOT / 'docs/index.md').read_text()
        self.assertRegex(text, r'https://github\.com/w1ne/labwired-core/blob/[0-9a-f]{40}/examples/luckfox-pico-mini-a-linux/BASKET\.md')


if __name__ == '__main__':
    unittest.main()
