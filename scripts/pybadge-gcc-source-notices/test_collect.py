import hashlib
import io
import tarfile
import unittest

from collect import BoundedReader, scan, selected_notice


def packet(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:") as archive:
        for name, content, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            if kind == tarfile.REGTYPE:
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
            else:
                member.linkname = "../../COPYING3"
                archive.addfile(member)
    return output.getvalue()


def inspect(raw, **limits):
    captures = {}

    def capture(name, body):
        captures[name] = body
        return {"capture": "notices/" + name, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()}

    return scan(io.BytesIO(raw), capture, **limits), captures


class StreamingAdmission(unittest.TestCase):
    def test_only_exact_names(self):
        for name in ("COPYING3", "gcc/COPYING.RUNTIME", "gcc/COPYING3.LIB", "COPYING"):
            self.assertTrue(selected_notice(name))
        for name in ("gcc/source.c", "COPYING3.c", "COPYING.RUNTIME-other", "copyright"):
            self.assertFalse(selected_notice(name))

    def test_stream_selects_notices_and_skips_implementation(self):
        raw = packet([("gcc/COPYING3", b"GPL candidate\n", tarfile.REGTYPE),
                      ("gcc/source.c", b"not a notice", tarfile.REGTYPE),
                      ("gcc/COPYING.RUNTIME", b"exception candidate\n", tarfile.REGTYPE)])
        report, captures = inspect(raw)
        self.assertEqual(report["memberCount"], 3)
        self.assertEqual(set(captures), {"gcc/COPYING3", "gcc/COPYING.RUNTIME"})
        self.assertEqual(captures["gcc/COPYING.RUNTIME"], b"exception candidate\n")
        self.assertEqual(report["missingRegularNoticeNames"], ["COPYING", "COPYING3.LIB"])

    def test_named_links_not_followed_or_counted_as_regular(self):
        raw = packet([("gcc/COPYING3", b"", tarfile.SYMTYPE),
                      ("gcc/lib/COPYING.RUNTIME", b"", tarfile.LNKTYPE)])
        report, captures = inspect(raw)
        self.assertEqual(captures, {})
        self.assertEqual(len(report["unresolvedNonregularNotices"]), 2)
        self.assertIn("COPYING3", report["missingRegularNoticeNames"])

    def test_normalized_duplicate_paths_rejected(self):
        raw = packet([("gcc/COPYING3", b"one", tarfile.REGTYPE),
                      ("./gcc/COPYING3", b"two", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            inspect(raw)

    def test_unsafe_paths_rejected(self):
        for name in ("../COPYING3", "/COPYING3", "gcc/../../COPYING3"):
            with self.assertRaises(ValueError):
                inspect(packet([(name, b"x", tarfile.REGTYPE)]))

    def test_member_count_bound(self):
        raw = packet([("a", b"x", tarfile.REGTYPE), ("b", b"x", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            inspect(raw, member_limit=1)

    def test_notice_size_bound(self):
        raw = packet([("COPYING3", b"12345", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            inspect(raw, notice_limit=4)

    def test_total_notice_bound(self):
        raw = packet([("COPYING3", b"123", tarfile.REGTYPE), ("COPYING.RUNTIME", b"456", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            inspect(raw, total_notice_limit=5)

    def test_expanded_stream_bound(self):
        raw = packet([("source.c", b"123", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            inspect(raw, expanded_limit=512)

    def test_member_payload_bound_before_skip(self):
        member = tarfile.TarInfo("source.c")
        member.size = 513 * 1024 * 1024
        with self.assertRaises(ValueError):
            inspect(member.tobuf() + b"\0" * 1024)

    def test_header_inventory_digest_deterministic(self):
        raw = packet([("COPYING3", b"one", tarfile.REGTYPE)])
        first, _ = inspect(raw)
        second, _ = inspect(raw)
        self.assertEqual(first["headerInventorySha256"], second["headerInventorySha256"])
        changed, _ = inspect(packet([("different", b"one", tarfile.REGTYPE)]))
        self.assertNotEqual(first["headerInventorySha256"], changed["headerInventorySha256"])

    def test_bounded_reader_exact_eof(self):
        reader = BoundedReader(io.BytesIO(b"abcd"), 4)
        self.assertEqual(reader.read(4), b"abcd")
        self.assertEqual(reader.read(1), b"")
        self.assertEqual(reader.count, 4)

    def test_bounded_reader_overrun_or_unbounded_rejected(self):
        with self.assertRaises(ValueError):
            BoundedReader(io.BytesIO(b"abcde"), 4).read(5)
        with self.assertRaises(ValueError):
            BoundedReader(io.BytesIO(b"abcd"), 4).read(-1)


if __name__ == "__main__":
    unittest.main()
