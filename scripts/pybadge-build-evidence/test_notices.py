from pathlib import Path
import unittest

from notices import leading_comments, package_owner


class Admission(unittest.TestCase):
    def test_exact_comment_bytes_stop_before_implementation(self):
        prefix = b"\xef\xbb\xbf\n/* Copyright original */\n// exception text\n/* next notice */"
        self.assertEqual(leading_comments(prefix + b"\n#define IMPLEMENTATION 1\n"), prefix)
        self.assertEqual(leading_comments(b"// notice without newline"), b"// notice without newline")

    def test_no_comment_is_not_a_clearance(self):
        for data in (b"", b" \n", b"#pragma once\n// later comment\n", b"int implementation;"):
            self.assertEqual(leading_comments(data), b"")

    def test_unterminated_or_oversized_comment_rejected(self):
        for data in (b"/* unfinished", b"// " + b"x" * 16384, b"/*" + b"x" * 16384 + b"*/"):
            with self.assertRaises(ValueError):
                leading_comments(data)

    def test_exact_package_owner_with_optional_architecture(self):
        path = Path("/usr/include/newlib/stdlib.h")
        for package in ("libnewlib-dev", "libnewlib-dev:amd64"):
            self.assertEqual(package_owner(package + ": " + str(path) + "\n", path), package)

    def test_missing_multiple_or_changed_ownership_rejected(self):
        path = Path("/usr/include/newlib/stdlib.h")
        for text in ("", "one: /usr/other.h", "one: " + str(path) + "\ntwo: " + str(path),
                     "one, two: " + str(path), "../../other: " + str(path)):
            with self.assertRaises(ValueError):
                package_owner(text, path)


if __name__ == "__main__":
    unittest.main()
