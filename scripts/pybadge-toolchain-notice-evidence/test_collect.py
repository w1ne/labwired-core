import io
import lzma
import tarfile
import unittest

from collect import notice_name, read_notices, source_url


def packet(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:xz") as archive:
        for name, content, kind in entries:
            member = tarfile.TarInfo(name)
            member.type = kind
            if kind == tarfile.REGTYPE:
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
            else:
                member.linkname = "other"
                archive.addfile(member)
    return output.getvalue()


class ArchiveAdmission(unittest.TestCase):
    def test_named_candidates(self):
        for name in ("debian/copyright", "debian/copyright-gcc", "COPYING.RUNTIME", "COPYING3.LIB", "LICENSE"):
            self.assertTrue(notice_name(name))

    def test_nonnotice_names_not_selected(self):
        for name in ("debian/rules", "source.c", "COPYING-other", "COPYING3.c", "copyright.in"):
            self.assertFalse(notice_name(name))

    def test_only_notice_members_are_returned(self):
        raw = packet([("debian/copyright", b"exact notice\n", tarfile.REGTYPE),
                      ("debian/rules", b"NOT A NOTICE", tarfile.REGTYPE)])
        inventory, notices = read_notices(raw)
        self.assertEqual(len(inventory), 2)
        self.assertEqual(notices, {"debian/copyright": b"exact notice\n"})

    def test_empty_or_absent_notices_remain_absent(self):
        _, notices = read_notices(packet([("debian/rules", b"other", tarfile.REGTYPE)]))
        self.assertEqual(notices, {})

    def test_directory_is_not_read_as_notice(self):
        _, notices = read_notices(packet([("LICENSE", b"", tarfile.DIRTYPE)]))
        self.assertEqual(notices, {})

    def test_unsafe_paths_rejected(self):
        for name in ("../copyright", "/copyright", "debian/../../copyright"):
            with self.assertRaises(ValueError):
                read_notices(packet([(name, b"x", tarfile.REGTYPE)]))

    def test_normalized_duplicate_paths_rejected(self):
        with self.assertRaises(ValueError):
            read_notices(packet([("debian/copyright", b"one", tarfile.REGTYPE),
                                 ("./debian/copyright", b"two", tarfile.REGTYPE)]))

    def test_links_and_special_members_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE, tarfile.CHRTYPE):
            with self.assertRaises(ValueError):
                read_notices(packet([("debian/copyright", b"", kind)]))

    def test_compressed_size_bound(self):
        with self.assertRaises(ValueError):
            read_notices(b"x" * (1024 * 1024 + 1))

    def test_member_count_bound(self):
        with self.assertRaises(ValueError):
            read_notices(packet([(f"file-{i}", b"", tarfile.REGTYPE) for i in range(257)]))

    def test_expanded_member_bound_before_member_body(self):
        member = tarfile.TarInfo("copyright")
        member.size = 5 * 1024 * 1024
        with self.assertRaises(ValueError):
            read_notices(lzma.compress(member.tobuf() + b"\0" * 1024))

    def test_notice_size_bound(self):
        member = tarfile.TarInfo("copyright")
        member.size = 1024 * 1024 + 1
        with self.assertRaises(ValueError):
            read_notices(lzma.compress(member.tobuf() + b"\0" * 1024))

    def test_malformed_archive(self):
        with self.assertRaises(ValueError):
            read_notices(b"not tar")

    def test_expansion_bound_including_tar_metadata(self):
        with self.assertRaises(ValueError):
            read_notices(lzma.compress(b"\0" * (4 * 1024 * 1024 + 1)))

    def test_truncated_or_trailing_xz_rejected(self):
        raw = packet([("copyright", b"notice", tarfile.REGTYPE)])
        for changed in (raw[:-4], raw + b"trailing", raw + raw):
            with self.assertRaises(ValueError):
                read_notices(changed)

    def test_official_archive_url(self):
        self.assertEqual(source_url("https://archive.ubuntu.com/ubuntu/pool/universe/n/newlib/a.dsc", "newlib.debian.tar.xz"),
                         "https://archive.ubuntu.com/ubuntu/pool/universe/n/newlib/newlib.debian.tar.xz")

    def test_unsafe_urls_rejected(self):
        for url, name in (("http://archive.ubuntu.com/ubuntu/pool/x/a.dsc", "a.tar.xz"),
                          ("https://other.invalid/ubuntu/pool/x/a.dsc", "a.tar.xz"),
                          ("https://archive.ubuntu.com/ubuntu/pool/../a.dsc", "a.tar.xz"),
                          ("https://archive.ubuntu.com/ubuntu/pool/x/a.dsc?query", "a.tar.xz"),
                          ("https://archive.ubuntu.com/ubuntu/pool/x/a.dsc", "../a.tar.xz"),
                          ("https://archive.ubuntu.com/ubuntu/pool/x/a.dsc", "a.tar.xz?query")):
            with self.assertRaises(ValueError):
                source_url(url, name)


if __name__ == "__main__":
    unittest.main()
