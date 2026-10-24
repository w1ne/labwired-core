"""Hosted-only original translation-unit preprocessing, never guest execution."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess

SCREEN_SHA = "dea9ea175d65d885275eb0715d56353674feae88d64d29a5eb2809f899a0d958"


def flags_from_make(text):
    result = []
    for key in ("CXX_DEFINES", "CXX_INCLUDES", "CXX_FLAGS"):
        matches = re.findall(r"^" + key + r" = (.*)$", text, re.MULTILINE)
        if len(matches) != 1 or "$" in matches[0] or matches[0].endswith("\\"):
            raise ValueError("unsupported or ambiguous generated flags: " + key)
        result.extend(shlex.split(matches[0]))
    return result


def require_rgb444(text):
    matches = re.findall(r"^#define USE_RGB444(?:[ \t]+(.*))?$", text, re.MULTILINE)
    if matches != ["1"]:
        raise ValueError("effective USE_RGB444 must be exactly 1")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compiler_from_record(text):
    matches = re.findall(r'^set\(CMAKE_CXX_COMPILER "([^"]+)"\)$', text, re.MULTILINE)
    if len(matches) != 1 or not Path(matches[0]).is_absolute() or Path(matches[0]).name != "arm-none-eabi-g++":
        raise ValueError("unexpected generated C++ compiler")
    return matches[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    source = root / "pxtapp/screen---st7735/screen.cpp"
    if digest(source.read_bytes()) != SCREEN_SHA:
        raise ValueError("original screen source identity mismatch")
    flags_file = root / "build/CMakeFiles/ITSYBITSY_M4.dir/flags.make"
    raw_flags = flags_file.read_bytes()
    flags = flags_from_make(raw_flags.decode())
    header = root / "build/codal_extra_definitions.h"
    forced = [flags[i + 1] for i, value in enumerate(flags[:-1]) if value == "-include"]
    if str(header) not in forced:
        raise ValueError("original forced configuration header missing from flags")
    compiler_records = list((root / "build/CMakeFiles").glob("*/CMakeCXXCompiler.cmake"))
    if len(compiler_records) != 1:
        raise ValueError("ambiguous generated C++ compiler record")
    compiler_record = compiler_records[0].read_bytes()
    compiler = compiler_from_record(compiler_record.decode())
    # Separate preprocessing with the actual target's generated flags; not a
    # claim that we captured the original compiler invocation or ran the guest.
    result = subprocess.run(
        [compiler, *flags, "-E", "-dM", str(source)], cwd=root / "build",
        check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60,
    )
    if len(result.stdout) > 4 * 1024 * 1024:
        raise ValueError("macro output exceeds capture limit")
    args.out.mkdir(parents=True, exist_ok=True)
    with (args.out / "screen-effective-macros.txt").open("xb") as output:
        output.write(result.stdout)
    with (args.out / "screen-preprocess-stderr.txt").open("xb") as output:
        output.write(result.stderr)
    # Preserve actual output even when the expected macro is absent/changed.
    require_rgb444(result.stdout.decode())
    evidence = {
        "schema": 1, "source": "pxtapp/screen---st7735/screen.cpp",
        "sourceSha256": SCREEN_SHA, "flagsSha256": digest(raw_flags),
        "forcedHeaderSha256": digest(header.read_bytes()),
        "compilerSha256": digest(Path(compiler).read_bytes()),
        "compilerRecordSha256": digest(compiler_record),
        "macroOutputSha256": digest(result.stdout), "effectiveUSE_RGB444": 1,
        "boundary": "Separate original ARM preprocessing with generated target flags; not loaded CF2, renderer execution or guest proof",
    }
    with (args.out / "screen-preprocess-evidence.json").open("x") as output:
        json.dump(evidence, output, indent=2)
        output.write("\n")
    print("Original ARM screen preprocessing PASS; effective USE_RGB444=1")


if __name__ == "__main__":
    main()
