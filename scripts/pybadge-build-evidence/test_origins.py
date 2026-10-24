import hashlib
import unittest

from origins import owner, request_bytes, require_bytes


class Admission(unittest.TestCase):
    def test_longest_component_owns_nested_submodule(self):
        self.assertEqual(owner("libraries/codal-samd/asf4/samd51/include/sam.h"),
                         ("libraries/codal-samd/asf4", "samd51/include/sam.h"))
        self.assertEqual(owner("libraries/codal-samd/samd-peripherals/samd/clock.c"),
                         ("libraries/codal-samd/samd-peripherals", "samd/clock.c"))
        self.assertEqual(owner("libraries/codal-samd/src/ZSPI.cpp"),
                         ("libraries/codal-samd", "src/ZSPI.cpp"))
        self.assertEqual(owner("utils/cmake/toolchains/ARM_GCC/platform_includes.h")[0], ".")

    def test_component_name_prefix_is_not_membership(self):
        self.assertEqual(owner("libraries/codal-samd/asf4-other/file.h")[0], "libraries/codal-samd")

    def test_noncanonical_paths_rejected(self):
        for name in ("../a", "/a", "a/../b", "a//b", "./a", "."):
            with self.subTest(name=name), self.assertRaises(ValueError):
                owner(name)

    def test_exact_request_byte_identity(self):
        request = {"replaceFiles": {"/pxtapp/header.h": "// original\n"}}
        data = request_bytes("pxtapp/header.h", request)
        record = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        require_bytes(data, record)
        with self.assertRaises(ValueError):
            require_bytes(data + b"\n", record)
        with self.assertRaises(ValueError):
            request_bytes("pxtapp/missing.h", request)


if __name__ == "__main__":
    unittest.main()
