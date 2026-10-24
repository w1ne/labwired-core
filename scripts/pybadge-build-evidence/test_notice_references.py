import gzip
from pathlib import Path
import unittest

from notice_references import decode_notice, missing_references, reference_tokens, selected_notice, verify_version


class Admission(unittest.TestCase):
    def test_exact_package_version(self):
        verify_version("libnewlib-dev", "1.2-3", b"libnewlib-dev\t1.2-3\n")
        verify_version("libnewlib-dev:amd64", "1.2-3", b"libnewlib-dev:amd64\t1.2-3\n")

    def test_changed_or_ambiguous_package_version_rejected(self):
        for raw in (b"other\t1.2-3\n", b"libnewlib-dev\t1.2-4\n", b"1.2-3\n",
                    b"libnewlib-dev\t1.2-3\nother\t1.2-3\n"):
            with self.assertRaises(ValueError):
                verify_version("libnewlib-dev", "1.2-3", raw)

    def test_exact_references_and_duplicates(self):
        self.assertEqual(reference_tokens(b"COPYING3 COPYING.RUNTIME COPYING3 COPYING3.LIB"),
                         ["COPYING.RUNTIME", "COPYING3", "COPYING3.LIB"])

    def test_selected_names_not_arbitrary_source(self):
        for name in ("copyright-gcc.gz", "COPYING.RUNTIME", "COPYING3", "copyright"):
            self.assertTrue(selected_notice(Path("/public/docs") / name))
        for name in ("source.cpp", "copyright-gcc.cpp", "README", "COPYING3-other"):
            self.assertFalse(selected_notice(Path("/public/docs") / name))

    def test_gzip_raw_and_decoded_bytes_distinct(self):
        raw = gzip.compress(b"exact notice\n")
        self.assertEqual(decode_notice(raw, True), b"exact notice\n")
        self.assertEqual(decode_notice(raw, False), raw)

    def test_oversized_or_invalid_compressed_notice_rejected(self):
        with self.assertRaises(ValueError):
            decode_notice(gzip.compress(b"x" * 100), True, limit=50)
        with self.assertRaises((OSError, EOFError)):
            decode_notice(b"not gzip", True)

    def test_missing_names_are_not_satisfied_by_copyright_candidates(self):
        self.assertEqual(missing_references(["COPYING3", "COPYING.RUNTIME"], ["copyright-gcc.gz"]),
                         ["COPYING3", "COPYING.RUNTIME"])
        self.assertEqual(missing_references(["COPYING3", "COPYING.RUNTIME"], ["COPYING3.gz"]),
                         ["COPYING.RUNTIME"])


if __name__ == "__main__":
    unittest.main()
