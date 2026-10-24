"""Separate dependency-only compiler probes; never original invocation evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess

from dependencies import origin, parse_dependencies


def sha(data):
    return hashlib.sha256(data).hexdigest()


def command_from_recipe(line, flags_text, build):
    """Admit only the observed generated command grammar, without a shell."""
    words = shlex.split(line)
    cwd = build
    if words[:1] == ["cd"]:
        if len(words) < 4 or words[2] != "&&":
            raise ValueError("unsupported recipe working directory")
        cwd = Path(words[1])
        words = words[3:]
    if not cwd.is_absolute() or cwd != cwd.resolve() or not cwd.resolve().is_relative_to(build.resolve()):
        raise ValueError("recipe working directory outside build")
    compilers = {"/usr/bin/arm-none-eabi-g++": "CXX", "/usr/bin/arm-none-eabi-gcc": "C"}
    if not words or words[0] not in compilers:
        raise ValueError("unreviewed compiler")
    language = compilers[words[0]]
    variables = [language + suffix for suffix in ("_DEFINES", "_INCLUDES", "_FLAGS")]
    if words[1:4] != ["$(" + key + ")" for key in variables]:
        raise ValueError("unsupported compiler variable ordering")
    tail = words[4:]
    if (len(tail) != 9 or tail[0:2] != ["-MMD", "-MT"] or
            tail[3:6] != ["-MF", "DEPFILE", "-o"] or tail[7] != "-c" or tail[2] != tail[6]):
        raise ValueError("unsupported generated compile recipe")
    target, source = tail[2], Path(tail[8])
    if Path(target).is_absolute() or ".." in Path(target).parts or not target.endswith(".o"):
        raise ValueError("unsupported object target")
    if not source.is_absolute() or source.suffix not in (".c", ".cpp"):
        raise ValueError("unsupported source language")
    flags = []
    for key in variables:
        matches = re.findall(r"^" + key + r" = (.*)$", flags_text, re.MULTILINE)
        if len(matches) != 1 or "$" in matches[0] or matches[0].endswith("\\"):
            raise ValueError("ambiguous generated flags")
        flags.extend(shlex.split(matches[0]))
    unsafe = ("@", "-o", "-M", "-fplugin", "-save-temps", "-specs", "--specs", "-wrapper", "-Wp,", "-X")
    if any(flag in ("-c", "-E", "-S") or flag.startswith(unsafe) for flag in flags):
        raise ValueError("output-affecting or executable compiler flag")
    return words[0], flags, cwd, source, target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.out.resolve()
    build = root / "build"
    image = build / "ITSYBITSY_M4"
    image_before = sha(image.read_bytes())
    records, files, identities, excluded = [], {}, {}, []
    total = rule_bytes = stderr_bytes = 0
    seen = set()
    for recipe in sorted(build.rglob("build.make")):
        raw_recipe = recipe.read_bytes()
        commands = [line for line in raw_recipe.decode().splitlines() if " -c " in line and "arm-none-eabi-" in line]
        if not commands:
            continue
        flags_file = recipe.parent / "flags.make"
        raw_flags = flags_file.read_bytes()
        for line in commands:
            if "$(ASM_DEFINES) $(ASM_INCLUDES) $(ASM_FLAGS)" in line:
                excluded.append({"recipe": recipe.relative_to(root).as_posix(), "command": line,
                                 "reason": "Assembly dependency coverage remains unqualified"})
                continue
            compiler, flags, cwd, source, target = command_from_recipe(line, raw_flags.decode(), build)
            if not source.resolve().is_relative_to(root) or source.is_symlink():
                raise ValueError("source outside original build root")
            key = (cwd.relative_to(build) / target).as_posix()
            if key in seen or len(seen) >= 2048:
                raise ValueError("duplicate or excessive diagnostic units")
            seen.add(key)
            if compiler not in identities:
                language = "CXX" if compiler.endswith("g++") else "C"
                compiler_records = list((build / "CMakeFiles").glob("*/CMake" + language + "Compiler.cmake"))
                if len(compiler_records) != 1:
                    raise ValueError("ambiguous compiler identity record")
                raw_record = compiler_records[0].read_bytes()
                matches = re.findall(r'^set\(CMAKE_' + language + r'_COMPILER "([^"]+)"\)', raw_record.decode(), re.MULTILINE)
                if matches != [compiler]:
                    raise ValueError("compiler recipe/identity mismatch")
                identities[compiler] = {"sha256": sha(Path(compiler).read_bytes()), "recordSha256": sha(raw_record)}
                record_output = out / "diagnostic-compiler-records" / compiler_records[0].name
                record_output.parent.mkdir(parents=True, exist_ok=True)
                with record_output.open("xb") as output:
                    output.write(raw_record)
            destination = out / "diagnostic-dependency-rules" / (key + ".d")
            destination.parent.mkdir(parents=True, exist_ok=True)
            if destination.exists():
                raise ValueError("diagnostic output already exists")
            source_before = sha(source.read_bytes())
            invocation = [compiler, *flags, "-M", "-MT", target, "-MF", str(destination), str(source)]
            result = subprocess.run(invocation, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            stderr = destination.with_suffix(".stderr.txt")
            stderr.write_bytes(result.stderr)
            stderr_bytes += len(result.stderr)
            if result.returncode or result.stdout or len(result.stderr) > 1024 * 1024 or stderr_bytes > 4 * 1024 * 1024:
                raise ValueError("diagnostic dependency compiler failed or unexpected output")
            if sha(source.read_bytes()) != source_before:
                raise ValueError("source changed during diagnostic probe")
            if destination.stat().st_size > 1024 * 1024:
                raise ValueError("diagnostic dependency rule exceeds bound")
            raw = destination.read_bytes()
            rule_bytes += len(raw)
            if rule_bytes > 16 * 1024 * 1024:
                raise ValueError("diagnostic raw rule capture exceeds bound")
            actual_target, dependencies = parse_dependencies(raw.decode())
            if actual_target != target:
                raise ValueError("diagnostic target mismatch")
            unit_files = []
            for dependency in dependencies:
                path = Path(dependency)
                if not path.is_absolute():
                    path = cwd / path
                category, name = origin(path, root)
                file_key = category + "/" + name
                unit_files.append(file_key)
                if file_key not in files:
                    size = path.stat().st_size
                    total += size
                    if size > 4 * 1024 * 1024 or total > 64 * 1024 * 1024 or len(files) >= 8192:
                        raise ValueError("diagnostic dependency inventory exceeds bound")
                    data = path.read_bytes()
                    if len(data) != size:
                        raise ValueError("dependency changed during diagnostic capture")
                    files[file_key] = {"bytes": size, "sha256": sha(data), "origin": category, "path": name,
                                       "containsVendorUsePhrase": b"Atmel microcontroller product" in data}
            records.append({"source": source.relative_to(root).as_posix(), "sourceSha256": source_before,
                            "target": key, "cwd": cwd.relative_to(root).as_posix(), "invocation": invocation,
                            "recipe": recipe.relative_to(root).as_posix(), "recipeSha256": sha(raw_recipe),
                            "flags": flags_file.relative_to(root).as_posix(), "flagsSha256": sha(raw_flags),
                            "capture": destination.relative_to(out).as_posix(), "ruleSha256": sha(raw),
                            "stderrSha256": sha(result.stderr), "dependencies": unit_files})
    if not records or not any(r["source"] == "pxtapp/screen---st7735/screen.cpp" for r in records):
        raise ValueError("missing diagnostic units or original screen")
    for key, record in files.items():
        path = (root if record["origin"] == "build-source" else Path("/usr")) / record["path"]
        if sha(path.read_bytes()) != record["sha256"]:
            raise ValueError("dependency changed across diagnostic probes")
    for record in records:
        if (sha((root / record["recipe"]).read_bytes()) != record["recipeSha256"] or
                sha((root / record["flags"]).read_bytes()) != record["flagsSha256"]):
            raise ValueError("generated compiler recipe or flags changed during probes")
    if any(sha(Path(compiler).read_bytes()) != identity["sha256"] for compiler, identity in identities.items()):
        raise ValueError("compiler changed during diagnostic probes")
    if sha(image.read_bytes()) != image_before:
        raise ValueError("original image changed during diagnostic probes")
    report = {"schema": 1, "units": records, "files": files, "compilers": identities, "excluded": excluded,
              "originalImageSha256BeforeAndAfter": image_before, "completeSourceToBinaryProof": False,
              "boundary": "Separate -M probes of observed generated C/C++ commands, including system headers; not original dependency output, retained-inline proof, assembly coverage, licensing clearance or guest execution"}
    with (out / "diagnostic-dependencies.json").open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(f"Separate dependency-only probes: {len(records)} units, {len(files)} files; original image unchanged")


if __name__ == "__main__":
    main()
