"""Conservative compiled dependency inventory, not retained-code clearance."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex


def parse_dependencies(text):
    joined = re.sub(r"\\\r?\n", " ", text)
    if "$" in joined or joined.count(":") != 1:
        raise ValueError("unsupported dependency rule syntax")
    target, _, body = joined.partition(":")
    targets = shlex.split(target)
    dependencies = shlex.split(body)
    if len(targets) != 1 or not targets[0].endswith(".o") or not dependencies:
        raise ValueError("missing or ambiguous compiled dependency rule")
    return targets[0], sorted(set(dependencies))


def origin(path, root):
    resolved = path.resolve()
    if resolved.is_relative_to(root):
        return "build-source", resolved.relative_to(root).as_posix()
    if resolved.is_relative_to(Path("/usr")):
        return "toolchain", resolved.relative_to(Path("/usr")).as_posix()
    raise ValueError("dependency outside reviewed build/toolchain roots")


def capture_discovery(build, root, out):
    """Retain generated dependency recipes even when the assumed format is absent."""
    names = {"DEPFILE", "depend.make", "compiler_depend.make", "depend.internal",
             "DependInfo.cmake", "build.make"}
    inventory, captured = [], []
    total = 0
    for path in sorted(build.rglob("*")):
        if path.is_symlink():
            continue
        if not path.is_file():
            continue
        if len(inventory) >= 16384:
            raise ValueError("build discovery file count exceeds bound")
        relative = path.relative_to(root).as_posix()
        size = path.stat().st_size
        inventory.append({"path": relative, "bytes": size})
        if path.name not in names:
            continue
        if size > 1024 * 1024 or total + size > 8 * 1024 * 1024:
            raise ValueError("generated dependency discovery exceeds bound")
        raw = path.read_bytes()
        if len(raw) != size:
            raise ValueError("generated dependency discovery changed during capture")
        total += size
        destination = out / "dependency-discovery" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as output:
            output.write(raw)
        captured.append({"path": relative, "bytes": size,
                         "sha256": hashlib.sha256(raw).hexdigest()})
    with (out / "dependency-discovery.json").open("x") as output:
        json.dump({"schema": 1, "inventory": inventory, "captured": captured,
                   "boundary": "Generated metadata and file names/sizes only; no dependency inference or binary capture"}, output, indent=2)
        output.write("\n")


def select_rules(build):
    rules = sorted(build.rglob("*.o.d"))
    if rules:
        return rules, "per-object GCC rules"
    # Actual CMake recipes from run37787234826 use -MF DEPFILE literally.
    # Each shared file can represent only its last compiler invocation.
    rules = sorted(build.rglob("DEPFILE"))
    for rule in rules:
        recipes = list(rule.parent.rglob("build.make"))
        if not any(" -MF DEPFILE " in recipe.read_text() for recipe in recipes):
            raise ValueError("shared DEPFILE lacks observed generated recipe binding")
    return rules, "shared literal DEPFILE; overwritten units unqualified"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root, out = args.root.resolve(), args.out.resolve()
    build = root / "build"
    out.mkdir(parents=True, exist_ok=True)
    capture_discovery(build, root, out)
    rules, mode = select_rules(build)
    if not 0 < len(rules) <= 2048:
        raise ValueError(f"missing or excessive GCC dependency files ({len(rules)}); generated discovery retained")
    files, units = {}, []
    total_bytes = 0
    rule_bytes = 0
    for rule in rules:
        if rule.is_symlink() or rule.stat().st_size > 1024 * 1024:
            raise ValueError("linked or oversized dependency file")
        raw = rule.read_bytes()
        rule_bytes += len(raw)
        if rule_bytes > 8 * 1024 * 1024:
            raise ValueError("raw dependency rules exceed capture bound")
        relative = rule.relative_to(root).as_posix()
        capture = out / "compiled-dependency-rules" / relative
        capture.parent.mkdir(parents=True, exist_ok=True)
        with capture.open("xb") as output:
            output.write(raw)
        target, dependencies = parse_dependencies(raw.decode())
        unit = {"rule": relative, "capture": "compiled-dependency-rules/" + relative,
                "ruleSha256": hashlib.sha256(raw).hexdigest(), "target": target, "dependencies": []}
        if Path(target).is_absolute() or ".." in Path(target).parts:
            raise ValueError("unexpected compiled target path")
        for dependency in dependencies:
            path = Path(dependency)
            if not path.is_absolute():
                path = (rule.parent if rule.name == "DEPFILE" else build) / path
            category, name = origin(path, root)
            key = category + "/" + name
            unit["dependencies"].append(key)
            if key in files:
                continue
            size = path.stat().st_size
            total_bytes += size
            if size > 4 * 1024 * 1024 or total_bytes > 64 * 1024 * 1024 or len(files) >= 8192:
                raise ValueError("source dependency inventory exceeds bound")
            data = path.read_bytes()
            if len(data) != size:
                raise ValueError("dependency changed during capture")
            # Presence is only a review hint. Absence never grants permission.
            files[key] = {"origin": category, "path": name, "bytes": size,
                          "sha256": hashlib.sha256(data).hexdigest(),
                          "containsVendorUsePhrase": b"Atmel microcontroller product" in data}
        units.append(unit)
    screen_observed = any("pxtapp/screen---st7735/screen.cpp.o" in unit["target"] for unit in units)
    if mode == "per-object GCC rules" and not screen_observed:
        raise ValueError("original screen translation unit missing from inventory")
    report = {"schema": 2, "mode": mode, "completeCompiledCoverage": False,
              "screenTranslationUnitObserved": screen_observed, "units": units, "files": files,
              "boundary": "Surviving original GCC rules only, including discarded code; shared DEPFILE overwrites, missing rules/assembly and retained headers/inline bytes remain unqualified; not licence decisions or complete source-to-binary proof"}
    with (out / "compiled-dependencies.json").open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(f"Compiled dependency inventory captured: {len(units)} units, {len(files)} unique files; no clearance inferred")


if __name__ == "__main__":
    main()
