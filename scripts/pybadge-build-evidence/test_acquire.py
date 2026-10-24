import hashlib
import io
import tarfile
import unittest

from acquire import select_members


def fixture(entries):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, data, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.size = len(data) if kind == tarfile.REGTYPE else 0
            member.linkname = "outside" if kind == tarfile.SYMTYPE else ""
            archive.addfile(member, io.BytesIO(data))
    return buffer.getvalue()


class Admission(unittest.TestCase):
    def test_selected_bytes_only_and_unselected_traversal_not_extracted(self):
        raw = fixture([("package/input", b"original\r\n", tarfile.REGTYPE),
                       ("../../ignored", b"no", tarfile.REGTYPE)])
        self.assertEqual(select_members(raw, hashlib.sha256(raw).hexdigest(),
                                        {"package/input": "input"}), {"input": b"original\r\n"})

    def test_changed_package_rejected(self):
        with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
            select_members(b"changed", "0" * 64, {})

    def test_missing_duplicate_and_selected_link_rejected(self):
        for entries in ([], [("package/input", b"", tarfile.SYMTYPE)],
                        [("package/input", b"a", tarfile.REGTYPE)] * 2):
            raw = fixture(entries)
            with self.assertRaises(ValueError):
                select_members(raw, hashlib.sha256(raw).hexdigest(), {"package/input": "input"})


if __name__ == "__main__":
    unittest.main()
