#!/usr/bin/env python3
"""Module dependency map of `crates/core/src`, and a ratchet on kernel->leaf edges.

WHY THIS EXISTS

Nearly every module in labwired-core imports nearly every other one. The bus,
the peripherals root and the generic device factory name vendor types, so a
vendor peripheral set cannot be moved into its own crate without first removing
those references. Nothing stopped that coupling from growing while the work to
reduce it was planned. This script measures it and fails when it grows.

WHAT IT DOES

  default          print the module graph as deterministic JSON
  --check          compare current kernel->leaf edges with the committed
                   baseline and report differences (new pairs, increases,
                   decreases, largest strongly connected component) to stdout
                   and $GITHUB_STEP_SUMMARY, with ::warning:: lines for new
                   pairs. Advisory: exit 0 unless the script or its JSON inputs
                   are broken.
  --check --strict the same, but exit 1 when a kernel module gained a leaf
                   dependency, an existing count grew, or a module is missing
                   from core_layers.json. Intended to be switched on once
                   vendor construction no longer lives in kernel modules.
  --update-baseline  rewrite the baseline from the current tree. Refuses to
                   raise any count or add any edge unless --allow-increase.

Nodes are the top-level modules of the crate, `peripherals/<dir>` for every
directory under `peripherals/` (files directly in `peripherals/` form the node
`peripherals`), and `cpu/<arch>` for the CPU files (grouped by file-name
prefix; `cpu` is the trait/root). Items defined in `lib.rs` are the node `lib`.
Edges are `crate::...`, `super::...` and `self::...` paths in production code,
counted once per resolved path occurrence. `use crate::{a::b, c}` groups are
expanded. `use child::...` in a `mod.rs`/`lib.rs` (including `pub use`
re-exports of a child module) counts as an edge to that child.

`kernel` and `leaf` are declared in `core_layers.json`. Modules in neither set
are neutral: their edges are reported but not ratcheted.

PRODUCTION LINES. Files under a `tests/` directory, `tests.rs`, `*_tests.rs`,
`tests_*.rs`, `*_test.rs`, files declared as `#[cfg(test)] mod x;`, and items
gated by `#[cfg(test)]` / `#[cfg(all(test, ..))]` or written as `mod tests`
are excluded from line counts and from edges. Lines are raw physical lines
(comments and blanks included).

KNOWN LIMITS (regex based, no cargo, no rustc)

  * Paths produced by macros (`macro_rules!`, `paste!`) are invisible, and
    `$crate::` paths inside macro bodies are not resolved.
  * `crate::Name` is attributed to `lib` for any name that is not a module,
    even when `lib.rs` merely re-exports it from another module.
  * `use child::x` outside `mod.rs`/`lib.rs`, glob imports (`use super::*`)
    and paths inside nested inline `mod {}` blocks resolve against the file's
    module, not the inline module.
  * `#[cfg(any(test, ..))]` and `#[cfg_attr(test, ..)]` are treated as
    production code.
  * Edges found through re-exports in `cpu/mod.rs` and `peripherals/mod.rs`
    are resolved by name from their `pub use` lines only.

Only the Python standard library is used; a run takes about a second.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from bisect import bisect_right
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DEFAULT_SRC = REPO / "crates" / "core" / "src"
DEFAULT_LAYERS = HERE / "core_layers.json"
DEFAULT_BASELINE = HERE / "core_layers_baseline.json"

CPU_ARCHES = [
    "aarch64", "avr", "cortex_a", "cortex_m", "mips", "pic16", "pic18",
    "pic24", "riscv", "rx", "xtensa", "jit_framework",
]

# ---------------------------------------------------------------------------
# Source masking: blank out comments, strings and char literals (length and
# newlines preserved) so brace matching and path regexes see only code.
# ---------------------------------------------------------------------------

_TOKEN = re.compile(
    r"//[^\n]*"
    r"|/\*"
    r"|(?<![A-Za-z0-9_])b?r(#*)\""
    r"|(?<![A-Za-z0-9_])b?\""
    r"|'(?:\\(?:x[0-9a-fA-F]{2}|u\{[^}]*\}|.)|[^\\'\n])'"
)
_BLOCK = re.compile(r"/\*|\*/")
_STR_END = re.compile(r'(?:\\.|[^"\\])*"', re.S)
_NOT_NL = re.compile(r"[^\n]")


def _blank(seg: str) -> str:
    return _NOT_NL.sub(" ", seg)


def mask(text: str) -> str:
    out = []
    pos = 0
    n = len(text)
    while pos < n:
        m = _TOKEN.search(text, pos)
        if not m:
            break
        out.append(text[pos:m.start()])
        tok = m.group(0)
        end = m.end()
        if tok.startswith("//"):
            pass
        elif tok == "/*":
            depth = 1
            p = end
            while depth and p < n:
                b = _BLOCK.search(text, p)
                if not b:
                    p = n
                    break
                depth += 1 if b.group(0) == "/*" else -1
                p = b.end()
            end = p
        elif m.group(1) is not None:
            closer = '"' + m.group(1)
            e = text.find(closer, end)
            end = n if e < 0 else e + len(closer)
        elif tok.endswith('"'):
            e = _STR_END.match(text, end)
            end = e.end() if e else n
        out.append(_blank(text[m.start():end]))
        pos = end
    out.append(text[pos:])
    return "".join(out)


# ---------------------------------------------------------------------------
# Test-code removal
# ---------------------------------------------------------------------------

_CFG_TEST = re.compile(
    r"#\s*!?\[\s*cfg\s*\(\s*(?:test|all\s*\(\s*test\b[^\]]*?\))\s*\)\s*\]"
)
_MOD_TESTS = re.compile(r"(?<![A-Za-z0-9_])mod\s+tests?\s*[{;]")
_ATTR = re.compile(r"\s*#\s*\[[^\]]*\]")
_CFG_TEST_MOD_DECL = re.compile(
    r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\s*\[[^\]]*\]\s*)*"
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)


def _item_end(t: str, j: int) -> int:
    depth = 0
    n = len(t)
    while j < n:
        ch = t[j]
        if ch in "{([":
            depth += 1
        elif ch in "})]":
            depth -= 1
            if depth <= 0 and ch == "}":
                return j + 1
        elif ch == ";" and depth == 0:
            return j + 1
        j += 1
    return n


def test_ranges(masked: str) -> list[tuple[int, int]]:
    ranges = []
    i = 0
    while True:
        m = _CFG_TEST.search(masked, i)
        m2 = _MOD_TESTS.search(masked, i)
        cands = [x for x in (m, m2) if x]
        if not cands:
            break
        first = min(cands, key=lambda x: x.start())
        j = first.end()
        if first is m:
            while True:
                a = _ATTR.match(masked, j)
                if not a:
                    break
                j = a.end()
            end = _item_end(masked, j)
        elif masked[first.end() - 1] == ";":
            end = first.end()
        else:
            end = _item_end(masked, first.end() - 1)
        ranges.append((first.start(), end))
        i = max(end, first.end())
    return ranges


def blank_ranges(masked: str, ranges: list[tuple[int, int]]) -> str:
    if not ranges:
        return masked
    parts = []
    pos = 0
    for a, b in ranges:
        if a < pos:
            continue
        parts.append(masked[pos:a])
        parts.append(_blank(masked[a:b]))
        pos = b
    parts.append(masked[pos:])
    return "".join(parts)


def count_test_lines(masked: str, ranges: list[tuple[int, int]]) -> int:
    nl = [i for i, c in enumerate(masked) if c == "\n"]
    covered: set[int] = set()
    for a, b in ranges:
        covered.update(range(bisect_right(nl, a - 1), bisect_right(nl, max(a, b - 1)) + 1))
    return len(covered)


# ---------------------------------------------------------------------------
# Path expansion
# ---------------------------------------------------------------------------

_IDENT_PATH = re.compile(r"\s*((?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*)")
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def expand(text: str) -> list[list[str]]:
    """Expand the path text following `crate::` into segment lists.

    `a::{b::c, d}` -> [[a,b,c],[a,d]]. Group extent is found by brace matching,
    so the text may continue past the end of the statement.
    """
    out: list[list[str]] = []

    def rec(prefix: list[str], t: str) -> None:
        m = _IDENT_PATH.match(t)
        segs = [x.strip() for x in m.group(1).split("::") if x.strip()]
        rest = t[m.end():]
        if rest.startswith("{"):
            depth = 0
            cur: list[str] = []
            items: list[str] = []
            for ch in rest[1:]:
                if ch == "{":
                    depth += 1
                elif ch == "}":
                    if depth == 0:
                        break
                    depth -= 1
                if ch == "," and depth == 0:
                    items.append("".join(cur))
                    cur = []
                else:
                    cur.append(ch)
            items.append("".join(cur))
            for it in items:
                if it.strip():
                    rec(prefix + segs, it)
        else:
            m2 = _IDENT.match(rest)
            if m2:
                segs = segs + [m2.group(0)]
            out.append(prefix + segs)

    rec([], text)
    return out


# ---------------------------------------------------------------------------
# Crate model
# ---------------------------------------------------------------------------


def is_test_path(rel: str) -> bool:
    parts = rel.split("/")
    base = parts[-1]
    return (
        "tests" in parts[:-1]
        or base in ("tests.rs", "tests_main.rs")
        or base.endswith("_tests.rs")
        or base.endswith("_test.rs")
        or base.startswith("tests_")
    )


def modpath(rel: str) -> list[str]:
    p = [x[:-3] if x.endswith(".rs") else x for x in rel.split("/")]
    if p[-1] == "mod":
        p = p[:-1]
    if p == ["lib"]:
        p = []
    return p


def cpu_node(stem: str) -> str:
    if stem in ("mod", "trait_def"):
        return "cpu"
    for p in CPU_ARCHES:
        if stem.startswith(p):
            return "cpu/" + p
    return "cpu/" + stem


class Crate:
    def __init__(self, src: Path):
        self.src = src
        self.top = set()
        for e in os.listdir(src):
            self.top.add(e[:-3] if e.endswith(".rs") else e)
        pdir = src / "peripherals"
        self.pdirs = {d for d in os.listdir(pdir) if (pdir / d).is_dir()} if pdir.is_dir() else set()
        cdir = src / "cpu"
        self.cpu_stems = (
            {e[:-3] if e.endswith(".rs") else e for e in os.listdir(cdir)} if cdir.is_dir() else set()
        )
        self.reexport: dict[str, dict[str, str]] = {}

    # node of an absolute module path
    def node_of(self, segs: list[str]) -> str:
        if not segs or segs[0] not in self.top or segs[0] == "lib":
            return "lib"
        a = segs[0]
        if len(segs) > 1 and segs[1] in self.reexport.get(a, {}):
            segs = [a, self.reexport[a][segs[1]]] + segs[2:]
        if a == "peripherals":
            if len(segs) > 1 and segs[1] in self.pdirs:
                return "peripherals/" + segs[1]
            return "peripherals"
        if a == "cpu":
            if len(segs) > 1 and segs[1] in self.cpu_stems:
                return cpu_node(segs[1])
            return "cpu"
        return a

    def file_node(self, rel: str) -> str:
        if rel == "lib.rs":
            return "lib"
        return self.node_of(modpath(rel))

    def collect_reexports(self) -> None:
        for d in ("cpu", "peripherals"):
            f = self.src / d / "mod.rs"
            if not f.is_file():
                continue
            t = mask(f.read_text(errors="replace"))
            t = blank_ranges(t, test_ranges(t))
            m_map: dict[str, str] = {}
            for m in re.finditer(r"(?<![A-Za-z0-9_])pub\s+use\s+", t):
                stmt = t[m.end():].split(";", 1)[0]
                for segs in expand_with_alias(stmt):
                    if len(segs) >= 2:
                        m_map[segs[-1]] = segs[0]
            self.reexport[d] = m_map


def expand_with_alias(stmt: str) -> list[list[str]]:
    """expand(), but an `X as Y` item yields Y as its final segment."""
    paths = expand(stmt)
    aliases = dict()
    for m in re.finditer(r"([A-Za-z_][A-Za-z0-9_]*)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)", stmt):
        aliases[m.group(1)] = m.group(2)
    out = []
    for p in paths:
        out.append(p[:-1] + [aliases.get(p[-1], p[-1])] if p and p[-1] in aliases else p)
    return out


def declared_test_files(src: Path, files: list[str]) -> set[str]:
    """Files declared as `#[cfg(test)] mod x;`, with everything below them."""
    excluded: set[str] = set()
    for rel in files:
        text = (src / rel).read_text(errors="replace")
        if "cfg(test)" not in text:
            continue
        t = mask(text)
        base = Path(rel).parent
        if Path(rel).name not in ("mod.rs", "lib.rs", "main.rs"):
            base = base / Path(rel).stem
        for m in _CFG_TEST_MOD_DECL.finditer(t):
            name = m.group(1)
            for cand in (base / f"{name}.rs", base / name / "mod.rs"):
                c = cand.as_posix()
                if (src / c).is_file():
                    excluded.add(c)
                    if cand.name == "mod.rs":
                        pref = cand.parent.as_posix() + "/"
                        excluded.update(f for f in files if f.startswith(pref))
    return excluded


_USE_STMT = re.compile(
    r"(?<![A-Za-z0-9_])(?:pub(?:\s*\([^)]*\))?\s+)?use\s+(?!crate\b|super\b|self\b|::)"
)
_REL = re.compile(r"(?<![A-Za-z0-9_:])(crate|super|self)\s*::\s*")


def file_refs(rel: str, masked: str, crate: Crate, child_names: set[str]):
    """Yield absolute segment lists referenced by production text of one file."""
    mp = modpath(rel)
    is_mod = rel.endswith("mod.rs") or rel == "lib.rs"
    for m in _REL.finditer(masked):
        kind = m.group(1)
        text = masked[m.end():m.end() + 4000]
        if kind == "crate":
            base: list[str] = []
        elif kind == "self":
            base = mp
        else:
            # count consecutive `super::` ending at m.end()
            start = m.start()
            k = 1
            pre = masked[:start]
            mm = re.search(r"((?:super\s*::\s*)+)$", pre)
            if mm:
                # preceded by more supers: this match belongs to the chain's
                # first element, which already accounted for it
                continue
            chain = re.match(r"((?:super\s*::\s*)+)", masked[m.start():])
            k = chain.group(1).count("super")
            text = masked[m.start() + chain.end():m.start() + chain.end() + 4000]
            base = mp[:-k] if k <= len(mp) else []
        for segs in expand(text):
            if segs:
                yield base + segs
    if is_mod:
        for m in _USE_STMT.finditer(masked):
            stmt = masked[m.end():m.end() + 4000]
            first = _IDENT.match(stmt.lstrip())
            if not first or first.group(0) not in child_names:
                continue
            stmt = stmt.split(";", 1)[0] if "{" not in stmt.split(";", 1)[0] else stmt
            for segs in expand(stmt):
                if segs:
                    yield mp + segs


def build_graph(src: Path) -> dict:
    src = Path(src)
    files = sorted(
        os.path.relpath(os.path.join(r, f), src).replace(os.sep, "/")
        for r, _d, fs in os.walk(src)
        for f in fs
        if f.endswith(".rs")
    )
    crate = Crate(src)
    crate.collect_reexports()
    excluded = declared_test_files(src, files)
    prod = defaultdict(int)
    tests = defaultdict(int)
    nfiles = defaultdict(int)
    edges: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    for rel in files:
        node = crate.file_node(rel)
        text = (src / rel).read_text(errors="replace")
        total = len(text.splitlines())
        if is_test_path(rel) or rel in excluded:
            tests[node] += total
            continue
        masked = mask(text)
        ranges = test_ranges(masked)
        if re.search(r"#\s*!\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", masked):
            tests[node] += total
            continue
        tl = count_test_lines(masked, ranges)
        prod[node] += total - tl
        tests[node] += tl
        nfiles[node] += 1
        code = blank_ranges(masked, ranges)
        pdir = (Path(rel).parent if Path(rel).name in ("mod.rs", "lib.rs") else Path(rel).parent / Path(rel).stem)
        child_names = set()
        d = src / pdir
        if d.is_dir():
            child_names = {e[:-3] if e.endswith(".rs") else e for e in os.listdir(d)}
        for segs in file_refs(rel, code, crate, child_names):
            dst = crate.node_of(segs)
            if dst != node:
                edges[node][dst] += 1
    names = sorted(set(prod) | set(tests) | set(edges) | {d for e in edges.values() for d in e})
    modules = {}
    for n in names:
        modules[n] = {
            "files": nfiles[n],
            "production_lines": prod[n],
            "test_lines": tests[n],
            "imports": {k: edges[n][k] for k in sorted(edges[n])},
        }
    sccs = strongly_connected(names, {n: list(edges[n]) for n in names})
    return {
        "version": 1,
        "summary": {
            "modules": len(names),
            "production_lines": sum(prod.values()),
            "largest_scc_modules": len(sccs[0]) if sccs else 0,
            "largest_scc_production_lines": sum(prod[n] for n in sccs[0]) if sccs else 0,
        },
        "modules": modules,
        "sccs": sccs,
    }


def strongly_connected(nodes: list[str], adj: dict[str, list[str]]) -> list[list[str]]:
    """Tarjan, iterative. Returns components with more than one node, largest first."""
    index: dict[str, int] = {}
    low: dict[str, int] = {}
    on: set[str] = set()
    stack: list[str] = []
    out: list[list[str]] = []
    counter = 0
    for root in nodes:
        if root in index:
            continue
        work = [(root, iter(sorted(adj.get(root, ()))))]
        index[root] = low[root] = counter
        counter += 1
        stack.append(root)
        on.add(root)
        while work:
            v, it = work[-1]
            advanced = False
            for w in it:
                if w not in index:
                    index[w] = low[w] = counter
                    counter += 1
                    stack.append(w)
                    on.add(w)
                    work.append((w, iter(sorted(adj.get(w, ())))))
                    advanced = True
                    break
                if w in on:
                    low[v] = min(low[v], index[w])
            if advanced:
                continue
            work.pop()
            if work:
                u = work[-1][0]
                low[u] = min(low[u], low[v])
            if low[v] == index[v]:
                comp = []
                while True:
                    w = stack.pop()
                    on.discard(w)
                    comp.append(w)
                    if w == v:
                        break
                if len(comp) > 1:
                    out.append(sorted(comp))
    out.sort(key=lambda c: (-len(c), c))
    return out


# ---------------------------------------------------------------------------
# Ratchet
# ---------------------------------------------------------------------------


def load_layers(path: Path) -> tuple[set[str], set[str]]:
    """Return (kernel, leaf). Raises on unparsable JSON or overlap."""
    d = json.loads(Path(path).read_text())
    kernel, leaf = set(d["kernel"]), set(d["leaf"])
    both = kernel & leaf
    if both:
        raise SystemExit(f"core_layers: modules in both kernel and leaf: {sorted(both)}")
    return kernel, leaf


def load_neutral(path: Path) -> set[str]:
    return set(json.loads(Path(path).read_text()).get("neutral", []))


def kernel_to_leaf(graph: dict, kernel: set[str], leaf: set[str]) -> dict[str, int]:
    out = {}
    for src, info in graph["modules"].items():
        if src not in kernel:
            continue
        for dst, c in info["imports"].items():
            if dst in leaf:
                out[f"{src} -> {dst}"] = c
    return dict(sorted(out.items()))


def compare(current: dict[str, int], baseline: dict[str, int]):
    """Return (violations, improvements) as lists of strings."""
    bad, good = [], []
    for edge, c in current.items():
        b = baseline.get(edge)
        if b is None:
            bad.append(f"new kernel->leaf dependency {edge} ({c})")
        elif c > b:
            bad.append(f"{edge} grew from {b} to {c}")
        elif c < b:
            good.append(f"{edge} fell from {b} to {c}")
    for edge, b in baseline.items():
        if edge not in current:
            good.append(f"{edge} is gone (was {b})")
    return bad, good


def check_unclassified(graph: dict, kernel: set[str], leaf: set[str], neutral: set[str] = frozenset()) -> list[str]:
    return sorted(n for n in graph["modules"] if n not in kernel and n not in leaf and n not in neutral)


def write_json(path: Path, data: dict) -> None:
    Path(path).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def run_check(graph, kernel, leaf, baseline_path, out=None, strict=False, summary_path=None, neutral=frozenset()) -> int:
    """Compare against the baseline.

    Report mode (default) prints the differences, emits `::warning::` lines for
    new kernel->leaf pairs, appends a markdown summary to `summary_path`, and
    returns 0. Strict mode returns 1 on any increase or unclassified module.
    Unparsable layers/baseline files raise and fail in both modes.
    """
    out = out or sys.stdout
    base = json.loads(Path(baseline_path).read_text())["kernel_to_leaf"]
    current = kernel_to_leaf(graph, kernel, leaf)
    bad, good = compare(current, base)
    s = graph["summary"]
    head = (
        f"core dependency map: {s['modules']} modules, {s['production_lines']} production lines; "
        f"largest strongly connected component: {s['largest_scc_modules']} modules, "
        f"{s['largest_scc_production_lines']} lines; kernel->leaf edges: {len(current)} "
        f"(sum {sum(current.values())}, baseline {len(base)} / {sum(base.values())})"
    )
    print(head, file=out)
    # A module absent from both lists is reported, not failed, in report mode:
    # adding a module is routine and should not block an unrelated PR.
    missing = check_unclassified(graph, kernel, leaf, neutral)
    lines = [f"### Core dependency map ({'strict' if strict else 'advisory'})", "", head, ""]
    if missing:
        print(f"note: modules not listed in core_layers.json (treated as neutral): {', '.join(missing)}", file=out)
        print(f"::warning::core_layers.json does not classify: {', '.join(missing)}", file=out)
        lines.append(f"Unclassified modules (treated as neutral): {', '.join(missing)}")
    if good:
        print("kernel->leaf dependencies decreased; lower the baseline with", file=out)
        print("  python3 scripts/ci/core_dep_map.py --update-baseline", file=out)
        lines.append("Decreases versus baseline:")
        for g in good:
            print(f"  - {g}", file=out)
            lines.append(f"- {g}")
    if bad:
        print("kernel modules gained dependencies on leaf modules versus the baseline:", file=out)
        lines.append("Increases versus baseline:")
        for b in bad:
            print(f"  - {b}", file=out)
            lines.append(f"- {b}")
            if b.startswith("new kernel->leaf"):
                print(f"::warning::{b}", file=out)
        print(
            "Prefer putting vendor-specific code behind a trait or in the leaf module. "
            + ("This is an error in strict mode." if strict else "This is advisory and does not fail the job."),
            file=out,
        )
    if not (bad or good or missing):
        lines.append("No change versus baseline.")
    if summary_path:
        with open(summary_path, "a") as f:
            f.write("\n".join(lines) + "\n")
    if strict and (bad or missing):
        return 1
    return 0


def update_baseline(graph, kernel, leaf, baseline_path, allow_increase) -> int:
    current = kernel_to_leaf(graph, kernel, leaf)
    p = Path(baseline_path)
    old = json.loads(p.read_text())["kernel_to_leaf"] if p.is_file() else None
    if old is not None and not allow_increase:
        bad, _ = compare(current, old)
        if bad:
            print("refusing to raise the baseline (pass --allow-increase to override):", file=sys.stderr)
            for b in bad:
                print(f"  - {b}", file=sys.stderr)
            return 1
    write_json(p, {"version": 1, "kernel_to_leaf": current})
    print(f"wrote {p} ({len(current)} edges, sum {sum(current.values())})")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--src", default=str(DEFAULT_SRC))
    ap.add_argument("--layers", default=str(DEFAULT_LAYERS))
    ap.add_argument("--baseline", default=str(DEFAULT_BASELINE))
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--strict", action="store_true", help="with --check: fail on increases")
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument("--allow-increase", action="store_true")
    ap.add_argument("--output", help="write the JSON graph here instead of stdout")
    a = ap.parse_args(argv)
    graph = build_graph(Path(a.src))
    if a.check or a.update_baseline:
        kernel, leaf = load_layers(Path(a.layers))
        if a.update_baseline:
            return update_baseline(graph, kernel, leaf, a.baseline, a.allow_increase)
        return run_check(
            graph, kernel, leaf, a.baseline, strict=a.strict, neutral=load_neutral(Path(a.layers)),
            summary_path=os.environ.get("GITHUB_STEP_SUMMARY"),
        )
    text = json.dumps(graph, indent=2, sort_keys=True) + "\n"
    if a.output:
        Path(a.output).write_text(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
