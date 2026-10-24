from pathlib import Path
import unittest

from diagnostic_dependencies import command_from_recipe


class Admission(unittest.TestCase):
    def setUp(self):
        self.build = Path("/public/build")
        self.flags = "CXX_DEFINES = -DORIGINAL=1\nCXX_INCLUDES = -I/public/inc\nCXX_FLAGS = -mcpu=cortex-m4 -include /public/build/config.h -O2\n"
        self.command = "/usr/bin/arm-none-eabi-g++ $(CXX_DEFINES) $(CXX_INCLUDES) $(CXX_FLAGS) -MMD -MT CMakeFiles/app.dir/screen.cpp.o -MF DEPFILE -o CMakeFiles/app.dir/screen.cpp.o -c /public/screen.cpp"

    def test_exact_cpp_and_c_commands(self):
        compiler, flags, cwd, source, target = command_from_recipe(self.command, self.flags, self.build)
        self.assertEqual(compiler, "/usr/bin/arm-none-eabi-g++")
        self.assertEqual(cwd, self.build)
        self.assertEqual(source, Path("/public/screen.cpp"))
        self.assertEqual(target, "CMakeFiles/app.dir/screen.cpp.o")
        self.assertIn("/public/build/config.h", flags)
        command = self.command.replace("g++", "gcc").replace("CXX_", "C_").replace(".cpp", ".c")
        self.assertEqual(command_from_recipe(command, self.flags.replace("CXX_", "C_"), self.build)[0], "/usr/bin/arm-none-eabi-gcc")

    def test_generated_cd_is_data_not_shell(self):
        command = "cd /public/build/libraries/core && " + self.command
        self.assertEqual(command_from_recipe(command, self.flags, self.build)[2], Path("/public/build/libraries/core"))

    def test_unreviewed_commands_rejected(self):
        commands = [self.command + " ; echo bad", self.command.replace("g++", "other"),
                    self.command.replace("-MF DEPFILE", "-MF changed"),
                    self.command.replace("$(CXX_INCLUDES)", "$(OTHER)"),
                    self.command.replace("-c /public/screen.cpp", "-c /public/startup.s"),
                    self.command.replace("-o CMakeFiles/app.dir/screen.cpp.o", "-o different.o"),
                    "cd /outside && " + self.command, "cd relative && " + self.command]
        for command in commands:
            with self.subTest(command=command), self.assertRaises(ValueError):
                command_from_recipe(command, self.flags, self.build)

    def test_unexpanded_duplicate_or_executable_flags_rejected(self):
        flags = [self.flags + "CXX_FLAGS = -O2\n", self.flags.replace("-O2", "$(FLAGS)"),
                 self.flags.replace("-O2", "-fplugin=/public/plugin.so"),
                 self.flags.replace("-O2", "-o /public/output"), self.flags.replace("-O2", "@response")]
        for text in flags:
            with self.subTest(flags=text), self.assertRaises(ValueError):
                command_from_recipe(self.command, text, self.build)


if __name__ == "__main__":
    unittest.main()
