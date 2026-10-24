import hashlib
import unittest

from collect import checksum_files, descriptor_url, exact_record, paragraphs, source_identity, verify_descriptor


class MetadataAdmission(unittest.TestCase):
    def test_control_continuations(self):
        self.assertEqual(paragraphs(b"Package: sample\nVersion: 1\nFiles:\n abc\n def\n"),
                         [{"Package": "sample", "Version": "1", "Files": "\nabc\ndef"}])

    def test_duplicate_field_or_orphan_rejected(self):
        for raw in (b"Package: sample\nPackage: other\n", b" continuation\n", b"not a field\n"):
            with self.assertRaises(ValueError):
                paragraphs(raw)

    def test_exact_record_not_latest_version(self):
        raw = b"Package: sample\nVersion: 2\n\nPackage: sample\nVersion: 1\n"
        self.assertEqual(exact_record(raw, "sample", "1")["Version"], "1")
        with self.assertRaises(ValueError):
            exact_record(raw, "sample", "3")

    def test_conflicting_exact_records_rejected(self):
        raw = b"Package: sample\nVersion: 1\nSource: first\n\nPackage: sample\nVersion: 1\nSource: second\n"
        with self.assertRaises(ValueError):
            exact_record(raw, "sample", "1")

    def test_source_version_explicit_and_implicit(self):
        record = {"Package": "binary", "Version": "1:2-3"}
        self.assertEqual(source_identity(record), ("binary", "1:2-3"))
        self.assertEqual(source_identity({**record, "Source": "source"}), ("source", "1:2-3"))
        self.assertEqual(source_identity({**record, "Source": "source (1:2-1)"}), ("source", "1:2-1"))

    def test_bad_source_identity_rejected(self):
        for source in ("../other", "source (1) extra", "source ()", "source (-- bad)"):
            with self.assertRaises(ValueError):
                source_identity({"Package": "binary", "Version": "1", "Source": source})

    def test_checksum_inventory(self):
        digest = "a" * 64
        self.assertEqual(checksum_files(digest + " 12 source_1.orig.tar.xz"),
                         {"source_1.orig.tar.xz": {"bytes": 12, "sha256": digest}})

    def test_unsafe_duplicate_malformed_checksums_rejected(self):
        digest = "a" * 64
        for value in ("", digest + " 12 ../escape", digest + " -1 name", "bad 12 name",
                      digest + " 12 name\n" + digest + " 12 name"):
            with self.assertRaises(ValueError):
                checksum_files(value)

    def test_official_descriptor_url_and_unsafe_paths(self):
        self.assertEqual(descriptor_url("pool/universe/g/gcc-arm-none-eabi", "gcc-arm-none-eabi_13.2.rel1-2.dsc"),
                         "https://archive.ubuntu.com/ubuntu/pool/universe/g/gcc-arm-none-eabi/gcc-arm-none-eabi_13.2.rel1-2.dsc")
        for directory, name in (("/pool/x", "a.dsc"), ("pool/../x", "a.dsc"),
                                ("other/x", "a.dsc"), ("pool/x", "../a.dsc")):
            with self.assertRaises(ValueError):
                descriptor_url(directory, name)

    def descriptor(self):
        body = "Source: sample\nVersion: 1:2-3\nChecksums-Sha256:\n " + "a" * 64 + " 12 sample.orig.tar.xz\n"
        raw = ("-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n" + body
               + "\n-----BEGIN PGP SIGNATURE-----\nSYNTHETIC\n-----END PGP SIGNATURE-----\n").encode()
        listed = {"sample.dsc": {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()},
                  "sample.orig.tar.xz": {"bytes": 12, "sha256": "a" * 64}}
        return raw, listed

    def test_descriptor_exact_identity_and_source_members(self):
        raw, listed = self.descriptor()
        self.assertEqual(verify_descriptor(raw, "sample", "1:2-3", listed, "sample.dsc"),
                         {"sample.orig.tar.xz": listed["sample.orig.tar.xz"]})

    def test_changed_descriptor_identity_or_member_rejected(self):
        raw, listed = self.descriptor()
        for data, package, version, inventory in (
                (raw + b"changed", "sample", "1:2-3", listed),
                (raw, "other", "1:2-3", listed), (raw, "sample", "1:2-4", listed),
                (raw, "sample", "1:2-3", {**listed, "extra.tar.xz": {"bytes": 1, "sha256": "b" * 64}})):
            with self.assertRaises(ValueError):
                verify_descriptor(data, package, version, inventory, "sample.dsc")

    def test_malformed_signed_descriptor_rejected(self):
        with self.assertRaises(ValueError):
            paragraphs(b"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nSource: sample\n")


if __name__ == "__main__":
    unittest.main()
