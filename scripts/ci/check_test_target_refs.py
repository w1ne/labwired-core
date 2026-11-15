#!/usr/bin/env python3
"""Fail when a CI lane, script or doc names a cargo test target that does not exist.

Integration tests of labwired-core are grouped: `tests/<group>/main.rs` is one
target `<group>` and every former per-file target is a module of it, selected
with `--test <group> <module>::`. A lane that still says `--test <old file>`
fails at run time with "no test target named ...", which no PR gate notices
because those lanes are path-filtered or run after merge. This check resolves
the real targets from `cargo metadata` and verifies every reference.

Checked, in .github/, scripts/, hil/, Makefiles, *.sh, *.toml and the docs
(historical records under docs/evidence, qa, superpowers, engineering,
performance and receipts are skipped):

  * `--test <name>` / `--test=<name>`: the target must exist in the package
    named by `-p/--package` in the same command (any workspace package when
    the command names none);
  * `--test "$var"` / `--test $var`: every value the file assigns to `var`
    (`var=value`, `for var in a b c`) must be an existing target;
  * `binary(<name>)` / `binary_id(<name>)` nextest filters;
  * a loop that takes its target from a data table (not resolvable from the
    file) is marked with a `test-target-refs: dynamic` comment above it;
  * `--test <group> <module>::...` and harness filters `<module>::` after `--`
    when the target is a grouped binary: `mod <module>;` must be declared in
    its main.rs.

Usage: python3 scripts/ci/check_test_target_refs.py [--check] [FILE ...]
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKIP_PREFIX = (
    "docs/evidence/", "docs/qa/", "docs/superpowers/", "docs/engineering/",
    "docs/performance/", "docs/receipts/", "CHANGELOG.md",
    "crates/core/tests/", "scripts/ci/check_test_target_refs.py",
    "scripts/ci/test_check_test_target_refs.py", "scripts/ci/test_workspace_test_shard.py",
)
SCAN_RE = re.compile(r"(^\.github/|^scripts/|^hil/|(^|/)Makefile[^/]*$|\.sh$|\.toml$|^docs/.*\.md$|^examples/.*\.md$|^[A-Z_]+\.md$)")
TEST_FLAG = re.compile(r"--test(?:=|\s+)(\"?)([A-Za-z0-9_$\{\}]+)\1")
BINARY_RE = re.compile(r"binary(?:_id)?\(\s*=?\s*([A-Za-z0-9_]+)\s*\)")
PKG_RE = re.compile(r"(?:-p|--package)[ =]\"?([A-Za-z0-9_-]+)")
DYNAMIC = "test-target-refs: dynamic"
CMD_START = re.compile(r"cargo(?:\s+\S+)*?\s+(?:test|nextest)\b|cargo-test-nonvacuous\.sh")


def load_targets(metadata: dict | None = None):
    """({package: {test target name: main.rs path or None}})"""
    if metadata is None:
        out = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT, check=True, capture_output=True, text=True,
        ).stdout
        metadata = json.loads(out)
    pkgs = {}
    for p in metadata["packages"]:
        tg = {}
        for t in p["targets"]:
            if "test" in t["kind"]:
                src = Path(t["src_path"])
                tg[t["name"]] = src if src.name == "main.rs" else None
        pkgs[p["name"]] = tg
    return pkgs


def modules_of(main_rs: Path) -> set[str]:
    try:
        text = main_rs.read_text(encoding="utf-8")
    except OSError:
        return set()
    return set(re.findall(r"^\s*(?:pub\s+)?mod\s+([A-Za-z0-9_]+)\s*;", text, re.M))


def window(lines: list[str], i: int) -> tuple[int, int]:
    """Line range of the command that line i belongs to."""
    a = i
    for k in range(i, max(i - 14, -1), -1):
        if CMD_START.search(lines[k]) and not lines[k].lstrip().startswith("#"):
            a = k
            break
        if k < i and not lines[k].strip():
            break
    b = i
    while b + 1 < len(lines):
        nxt = lines[b + 1].strip()
        if lines[b].rstrip().endswith(("\\", "&&")) or re.match(r"^(--?[A-Za-z]|&&)", nxt):
            b += 1
        else:
            break
    return a, b


def var_values(text: str, var: str) -> set[str]:
    vals = set(re.findall(r"(?<![A-Za-z0-9_])%s=([A-Za-z0-9_]+)" % re.escape(var), text))
    for m in re.finditer(r"for\s+%s\s+in\s+([^;\n]+)" % re.escape(var), text):
        vals |= {v for v in m.group(1).split() if re.fullmatch(r"[A-Za-z0-9_]+", v)}
    return vals


def check_text(path: str, text: str, pkgs) -> list[str]:
    errs: list[str] = []
    lines = text.split("\n")
    every = set().union(*[set(t) for t in pkgs.values()]) if pkgs else set()
    is_doc = path.endswith(".md")
    seen: set[str] = set()
    for i, line in enumerate(lines):
        if "--test" not in line and "binary(" not in line and "binary_id(" not in line:
            continue
        if re.search(r"\bnode\s+--test\b", line) or line.lstrip().startswith(("#", "//")) and not is_doc:
            continue
        if any(DYNAMIC in l for l in lines[max(0, i - 6):i + 1]):
            continue
        a, b = window(lines, i)
        cmd = " ".join(lines[a:b + 1])
        # the segment of the command this line belongs to (split at && ; ||)
        seg = line
        for part in re.split(r"&&|;|\|\|", cmd):
            if line.strip()[:40] and line.strip()[:40] in part or (not seg.strip()):
                seg = part
                break
        pm = PKG_RE.search(seg) or PKG_RE.search(cmd)
        scope = pkgs.get(pm.group(1)) if pm else None
        known = set(scope) if scope is not None else every
        where = f"{path}:{i + 1}"

        def bad(name, why="no test target named", where=where):
            msg = f"{where}: {why} `{name}`" + (f" in package `{pm.group(1)}`" if pm else "")
            if msg not in seen:
                seen.add(msg)
                errs.append(msg)

        grouped = []
        for m in TEST_FLAG.finditer(seg):
            name = m.group(2)
            if "{" in name and not name.startswith("$"):
                continue
            if name.startswith("$"):
                var = name.strip("${}")
                vals = var_values(text, var)
                if not vals:
                    bad(name, f"cannot resolve; assign `{var}=<target>` in this file or mark the loop with `{DYNAMIC}`:")
                for v in sorted(vals):
                    if v not in known:
                        bad(v, f"`{name}` can be `{v}`, which is not a test target:")
                continue
            if name not in known:
                bad(name)
                continue
            main_rs = next((p[name] for p in ([scope] if scope is not None else pkgs.values()) if name in p and p[name]), None)
            if main_rs is not None:
                grouped.append((name, main_rs))
        if grouped:
            filters = re.findall(r"(?<![A-Za-z0-9_:$\{])([A-Za-z0-9_]+)::", re.sub(r"--test(?:=|\s+)\"?[A-Za-z0-9_$\{\}]+\"?", "", seg))
            if len(grouped) > 1 and filters:
                bad(", ".join(n for n, _ in grouped), "name filters apply to every selected binary, but this command selects several grouped targets:")
            for v in re.findall(r"\$\{?([A-Za-z0-9_]+)\}?::", seg):
                vals = var_values(text, v)
                if not vals:
                    bad(f"${v}::", f"cannot resolve; assign `{v}=<module>` in this file or mark the loop with `{DYNAMIC}`:")
                for val in sorted(vals):
                    if not any(val in modules_of(m) for _, m in grouped):
                        bad(f"{val}::", f"`${v}::` can be `{val}`, which is not a module of grouped target `{grouped[0][0]}`:")
            for f in filters:
                if not any(f in modules_of(m) for _, m in grouped):
                    bad(f"{f}::", f"not a module of grouped target `{grouped[0][0]}`:")
        for m in BINARY_RE.finditer(line):
            if m.group(1) not in known:
                bad(m.group(1), "nextest binary():")
    return errs


def check_data(pkgs) -> list[str]:
    """Targets named in the shard classification must exist."""
    errs = []
    p = ROOT / "scripts/ci/workspace-test-shards.json"
    if p.is_file():
        d = json.loads(p.read_text(encoding="utf-8"))
        for key, rows in d.items():
            for e in rows if isinstance(rows, list) else []:
                if isinstance(e, dict) and "package" in e and "target" in e:
                    if e["target"] not in pkgs.get(e["package"], {}):
                        errs.append(f"scripts/ci/workspace-test-shards.json: {key}: no test target `{e['target']}` in `{e['package']}`")
    return errs


def scan_files() -> list[str]:
    out = subprocess.run(["git", "ls-files"], cwd=ROOT, check=True, capture_output=True, text=True).stdout.split("\n")
    return [f for f in out if f and SCAN_RE.search(f) and not f.startswith(SKIP_PREFIX) and not f.endswith("Cargo.toml") and not f.endswith("Cargo.lock")]


def main(argv: list[str]) -> int:
    files = [a for a in argv if a != "--check"] or scan_files()
    pkgs = load_targets()
    errs = []
    for f in files:
        try:
            text = (ROOT / f).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        errs += check_text(f, text, pkgs)
    errs += check_data(pkgs)
    n_targets = sum(len(t) for t in pkgs.values())
    if errs:
        print("\n".join(errs), file=sys.stderr)
        print(f"\n{len(errs)} reference(s) to missing test targets/modules ({n_targets} targets known)", file=sys.stderr)
        return 1
    print(f"ok: {len(files)} files, every test-target reference resolves ({n_targets} targets)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
