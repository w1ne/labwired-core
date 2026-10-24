import unittest

from link_map import original_map_recipe

COMPILER = "/usr/bin/arm-none-eabi-g++"
RECIPE = COMPILER + ' "object with spaces.o" -T"/public/source/ld/script.ld" -Wl,-Map,ITSYBITSY_M4.map -o ITSYBITSY_M4 lib.a\n'


class Admission(unittest.TestCase):
    def test_observed_original_map_recipe(self):
        self.assertEqual(original_map_recipe(RECIPE, COMPILER),
                         ("ITSYBITSY_M4", "ITSYBITSY_M4.map", "/public/source/ld/script.ld"))

    def test_ambiguous_shell_response_and_changed_output_rejected(self):
        for mutant in (RECIPE + RECIPE, RECIPE.strip() + " && echo done", RECIPE.strip() + " @inputs",
                       RECIPE.replace("-o ITSYBITSY_M4", "-o other"), RECIPE.strip() + " -o second",
                       RECIPE.strip() + " $(FLAGS)", RECIPE.replace(COMPILER, "/usr/bin/g++")):
            with self.assertRaises(ValueError):
                original_map_recipe(mutant, COMPILER)

    def test_missing_duplicate_and_alternative_map_or_script_rejected(self):
        for mutant in (RECIPE.replace("-Wl,-Map,ITSYBITSY_M4.map", ""),
                       RECIPE.replace("-Map,ITSYBITSY_M4.map", "-Map=other.map"),
                       RECIPE.strip() + " -Wl,-Map,second.map", RECIPE.strip() + " --cref",
                       RECIPE.replace('-T"/public/source/ld/script.ld"', ""),
                       RECIPE.strip() + " -Tsecond.ld"):
            with self.assertRaises(ValueError):
                original_map_recipe(mutant, COMPILER)


if __name__ == "__main__":
    unittest.main()
