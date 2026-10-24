import unittest

from preprocess import compiler_from_record, flags_from_make, require_rgb444


class Admission(unittest.TestCase):
    def test_generated_compiler_identity(self):
        good = 'set(CMAKE_CXX_COMPILER "/usr/bin/arm-none-eabi-g++")\n'
        self.assertEqual(compiler_from_record(good), "/usr/bin/arm-none-eabi-g++")
        for mutant in ("", good + good, good.replace("arm-none-eabi-g++", "g++"), good.replace("/usr/bin/", "")):
            with self.assertRaises(ValueError):
                compiler_from_record(mutant)

    def test_generated_flags_preserve_quoted_arguments(self):
        raw = 'CXX_DEFINES = -DFOO=1\nCXX_INCLUDES = -I"a b"\nCXX_FLAGS = -include /public/config.h -O2\n'
        self.assertEqual(flags_from_make(raw), [
            "-DFOO=1", "-Ia b", "-include", "/public/config.h", "-O2"
        ])

    def test_ambiguous_missing_and_unexpanded_flags_rejected(self):
        valid = "CXX_DEFINES = \nCXX_INCLUDES = \nCXX_FLAGS = -O2\n"
        for mutant in (
            valid + "CXX_FLAGS = -O0\n",
            valid.replace("CXX_FLAGS = -O2\n", ""),
            valid.replace("-O2", "$(EXTRA)"),
            valid.replace("-O2", "-O2\\"),
        ):
            with self.assertRaises(ValueError):
                flags_from_make(mutant)

    def test_exact_effective_macro(self):
        require_rgb444("#define OTHER 2\n#define USE_RGB444 1\n")

    def test_absent_disabled_or_duplicate_macro_rejected(self):
        for mutant in (
            "", "#define USE_RGB444 0\n", "#define USE_RGB444\n",
            "#define USE_RGB444 1\n#define USE_RGB444 1\n",
        ):
            with self.assertRaises(ValueError):
                require_rgb444(mutant)


if __name__ == "__main__":
    unittest.main()
