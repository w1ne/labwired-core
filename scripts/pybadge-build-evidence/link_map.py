"""Capture the observed original SAMD link map without relinking or execution."""
import argparse
import hashlib
import json
from pathlib import Path
import shlex

from preprocess import compiler_from_record


def original_map_recipe(text, compiler):
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if len(lines) != 1 or any(char in lines[0] for char in "$`\n"):
        raise ValueError("unsupported generated link command")
    command = shlex.split(lines[0])
    if not command or command[0] != compiler:
        raise ValueError("link/compiler identity mismatch")
    if any(token in (";", "&&", "||", "|", ">", "<") or token.startswith("@") for token in command):
        raise ValueError("shell/response link recipes are unsupported")
    outputs = [i for i, token in enumerate(command) if token == "-o"]
    if len(outputs) != 1 or outputs[0] + 1 == len(command) or command[outputs[0] + 1] != "ITSYBITSY_M4":
        raise ValueError("unexpected original link output")
    maps = [token for token in command if "-Map" in token or token == "--cref"]
    if maps != ["-Wl,-Map,ITSYBITSY_M4.map"]:
        raise ValueError("unexpected original map option; explicit review required")
    scripts = [token[2:] for token in command if token.startswith("-T")]
    if len(scripts) != 1:
        raise ValueError("unexpected original linker script")
    return "ITSYBITSY_M4", "ITSYBITSY_M4.map", scripts[0]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.out.resolve()
    build = root / "build"
    out.mkdir(parents=True, exist_ok=True)
    raw = (build / "CMakeFiles/ITSYBITSY_M4.dir/link.txt").read_bytes()
    with (out / "original-link.txt").open("xb") as capture:
        capture.write(raw)
    records = list((build / "CMakeFiles").glob("*/CMakeCXXCompiler.cmake"))
    if len(records) != 1:
        raise ValueError("ambiguous generated compiler record")
    compiler = compiler_from_record(records[0].read_text())
    elf_name, map_name, script_name = original_map_recipe(raw.decode(), compiler)
    script = Path(script_name)
    expected_script = root / "libraries/codal-itsybitsy-m4/ld/samd51g19a_flash.ld"
    if script != expected_script or script.is_symlink():
        raise ValueError("unexpected linker script source")
    elf, map_file = build / elf_name, build / map_name
    if elf.is_symlink() or map_file.is_symlink() or not elf.is_file() or not map_file.is_file():
        raise ValueError("missing or linked original outputs")
    if not 0 < map_file.stat().st_size <= 8 * 1024 * 1024:
        raise ValueError("map capture size outside bound")
    elf_sha = file_digest(elf)
    data, script_data = map_file.read_bytes(), script.read_bytes()
    for name, body in (("original-link.map", data), ("original-linker-script.ld", script_data)):
        with (out / name).open("xb") as capture:
            capture.write(body)
    report = {
        "schema": 2, "mode": "original-build-map-no-relink",
        "originalLinkSha256": digest(raw), "originalElfSha256": elf_sha,
        "mapSha256": digest(data), "mapBytes": len(data),
        "linkerScript": "libraries/codal-itsybitsy-m4/ld/samd51g19a_flash.ld",
        "linkerScriptSha256": digest(script_data), "originalUnchanged": file_digest(elf) == elf_sha,
        "boundary": "Original clean-build recipe/map observation; no relink, firmware execution, independent binary/map replay or licence clearance",
    }
    with (out / "original-link-evidence.json").open("x") as capture:
        json.dump(report, capture, indent=2)
        capture.write("\n")
    if not report["originalUnchanged"]:
        raise ValueError("original ELF changed during read-only capture")
    print("Original clean-build map captured; no relink or firmware execution")


if __name__ == "__main__":
    main()
