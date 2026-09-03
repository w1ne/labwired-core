#!/usr/bin/env python3
"""The host crates clippy never linted — derived from the tree, not listed.

`cargo clippy --all-targets` at the workspace root does NOT lint the workspace.
It resolves to `default-members`, which is five packages of eighty-four:

    ["crates/cli", "crates/core", "crates/dap", "crates/loader",
     "crates/labwired-fuzz"]

Most of the other seventy-nine are firmware: `no_std` crates that only build for
thumbv*/riscv32*/xtensa*, and pointing a host clippy at them is meaningless. But
not all of them are. Twelve are ordinary host crates, and what reaches them is
partial in a way that is easy to mis-state, so — measured on this tree:

  * cargo applies `RUSTC_WORKSPACE_WRAPPER=clippy-driver` to every WORKSPACE
    member it builds, dependencies included. So six of the twelve already get
    their **lib** linted incidentally, as path-dependencies of a default-member:
    labwired-codegen, labwired-config, labwired-gdbstub, labwired-hw-trace,
    labwired-ir, svd-ingestor.
  * The other six are reached by no clippy lane at all, in any target:
    labwired-egress-relay, labwired-hw-oracle, labwired-hw-oracle-macros,
    labwired-hw-runner, validation-report, labwired-python. That includes
    `labwired-hw-oracle`, the silicon ground-truth comparator.
  * And for ALL twelve, everything that is not the lib — `tests/`, `benches/`,
    `[[bin]]` targets, `#[cfg(test)]` modules — is linted by nothing, because a
    dependency is only ever built as a lib. That is precisely the class the note
    on pr-gate's own clippy step already records: `channel: Option<String>` on
    `BoardIoBinding` stranded four literals in the DAP adapter's tests and "was
    invisible to anything short of `--all-targets`".

None of this is a testing gap. `pr-workspace-tests` shards
`cargo test --workspace --no-run`, which compiles and runs every member. It is
clippy, and only clippy, that stopped short.

`crates/wasm` was in exactly this state and was rescued one crate at a time: it
got its own `-p labwired-wasm` clippy step in `browser-layer` after a
browser-only fork shipped behind a fully green board, and the note on that step
ends "Before this step, NO pre-merge lane in this repo compiled the code the
browser actually runs." That argument was right and it was never generalized —
it applies verbatim to every host crate outside `default-members`.

A longer `-p` list in the workflow would be the same defect one level up: the
hole reopens the moment somebody adds a crate. So the scope is DERIVED. A member
is cross-target-only when it says so itself, in one of the two ways this
repository already uses:

  1. a per-crate `.cargo/config.toml` pinning `[build] target` to a bare-metal
     triple (22 members do), or
  2. a crate root carrying `#![no_std]` (65 members do).

Everything else is host-lintable. Of those, `default-members` are already
covered by the bare `cargo clippy --all-targets` and `labwired-wasm` by its own
step, so what this script emits is the remainder: the crates nothing lints. Add
a host crate to `members` and it is linted from the next run on, with nobody
editing a list.

⚠️ The `.cargo/config.toml` target is read as a DECLARATION, not as the
mechanism. Cargo reads config from the invocation's cwd and its ancestors, so
`cargo clippy -p firmware-rp2040-demo` run from the workspace root never sees
`crates/firmware-rp2040-demo/.cargo/config.toml` at all and would try to build
that crate for the host. The file is still the crate's own statement of which
target it is for, which is exactly what needs classifying here.

USAGE

    python3 scripts/ci/host_clippy_scope.py --check       # assert the rule holds
    python3 scripts/ci/host_clippy_scope.py --cargo-args  # one --package= per line
    python3 scripts/ci/host_clippy_scope.py --json

The clippy step builds its invocation from `--cargo-args`, so it cannot fall
behind the tree: it holds no list to fall behind with. `--check` closes the
other half — it asserts that the three scopes TOGETHER (the bare
default-members command, the `-p labwired-wasm` step, and this one) still cover
every host-lintable member, so no crate can fall between them either.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# `#![no_std]`, and the conditional spelling in
# crates/firmware-nrf52840-obd2-scanner: `#![cfg_attr(not(test), no_std)]`.
# A crate that is no_std under ANY configuration is firmware, not a host crate.
NO_STD = re.compile(r"^\s*#!\[[^]]*\bno_std\b[^]]*\]", re.M)

# Bare-metal / non-host triples: thumbv7em-none-eabi, riscv32imac-unknown-none-elf,
# xtensa-esp32s3-none-elf, wasm32-unknown-unknown.
OFF_HOST_TRIPLE = re.compile(r"-none(?:-|$)|^wasm\d*-")

CLIPPY_LINE = re.compile(r"\bcargo\s+clippy\b")
PACKAGE_ARG = re.compile(r"(?:-p|--package)[=\s]+\"?'?([A-Za-z0-9_.-]+)")

# ── The one explicit exception ────────────────────────────────────────────────
#
# `labwired-wasm` IS host-lintable — this script classifies it as such — and it
# IS linted pre-merge. It is simply linted somewhere else, by the dedicated
# `-p labwired-wasm` step in `browser-layer`, and it stays there for a reason
# that is not tidiness:
#
#   FEATURE UNIFICATION. `crates/wasm` turns `event-scheduler` on, and one cargo
#   invocation unifies features across every package in it. Naming wasm
#   alongside `labwired-core` therefore lints a DIFFERENT core from the one the
#   browser builds. Measured on this tree: folding wasm into this scope turns
#   the step red on `crates/core/tests/esp32_classic_walk_differential.rs:89`
#   (clippy::type_complexity) — real debt in the event-scheduler arm that no
#   lane lints, and not debt this scope should be paying by accident. Its own
#   invocation keeps the browser's exact feature set, which is the point of
#   that job.
#
# `--check` asserts a `cargo clippy` step in .github/workflows/ still names this
# package. An exception whose justification has quietly evaporated is the same
# hole with better paperwork, so it fails closed instead of trusting this
# comment.
BROWSER_LAYER_PACKAGE = "labwired-wasm"


class Unclassified(RuntimeError):
    """A member neither signal can place. Fails the gate rather than guessing."""


def workspace(root: Path) -> dict:
    return tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]


def _crate_roots(crate_dir: Path, manifest: dict) -> list[Path]:
    """Every source file cargo would compile as a crate root for this package.

    Read from the manifest, not guessed: `firmware-f407-demo` declares two
    `[[bin]]` targets at `src/smoke.rs` and `src/i2c.rs` and has no
    `src/main.rs`, so a scan that only looked at the default paths would find no
    `#![no_std]` and call a Cortex-M4 firmware crate a host crate.
    """
    candidates: list[Path] = [
        crate_dir / manifest.get("lib", {}).get("path", "src/lib.rs"),
        crate_dir / "src" / "main.rs",
    ]
    for entry in manifest.get("bin") or []:
        candidates.append(
            crate_dir / entry.get("path", f"src/bin/{entry.get('name', '')}.rs")
        )
    candidates.extend(sorted(crate_dir.glob("src/bin/*.rs")))
    roots: list[Path] = []
    for path in candidates:
        if path.is_file() and path not in roots:
            roots.append(path)
    return roots


def _pinned_target(crate_dir: Path) -> str | None:
    """`[build] target` from the crate's own .cargo/config.toml, if any."""
    config = crate_dir / ".cargo" / "config.toml"
    if not config.is_file():
        return None
    target = tomllib.loads(config.read_text(encoding="utf-8")).get("build", {}).get("target")
    if isinstance(target, list):  # cargo allows a list of triples
        return target[0] if target else None
    return target


def classify(root: Path) -> tuple[list[dict], list[dict]]:
    """Split `workspace.members` into (host-lintable, cross-target-only).

    Each entry carries the REASON it landed where it did, so a surprise shows up
    as a wrong reason rather than as a silent membership change.
    """
    host: list[dict] = []
    cross: list[dict] = []

    for member in workspace(root)["members"]:
        crate_dir = root / member
        manifest = tomllib.loads((crate_dir / "Cargo.toml").read_text(encoding="utf-8"))
        name = manifest["package"]["name"]

        target = _pinned_target(crate_dir)
        if target is not None:
            if not OFF_HOST_TRIPLE.search(target):
                raise Unclassified(
                    f'{member}: .cargo/config.toml pins `target = "{target}"`, '
                    f"which does not look like a bare-metal triple. Decide "
                    f"explicitly whether `{name}` is host-lintable and either "
                    f"widen OFF_HOST_TRIPLE or name it as an exception here — "
                    f"guessing is how a crate falls out of clippy silently."
                )
            cross.append({"member": member, "package": name, "reason": f"builds for {target}"})
            continue

        roots = _crate_roots(crate_dir, manifest)
        if not roots:
            raise Unclassified(
                f"{member}: no crate root found for `{name}`. Neither signal can "
                f"be read, so this member cannot be classified."
            )
        no_std = [
            p
            for p in roots
            if NO_STD.search(p.read_text(encoding="utf-8", errors="replace"))
        ]
        if no_std:
            cross.append(
                {
                    "member": member,
                    "package": name,
                    "reason": f"#![no_std] in {no_std[0].relative_to(root)}",
                }
            )
            continue

        host.append({"member": member, "package": name, "reason": "host crate"})

    return host, cross


def clippy_packages(root: Path) -> list[str]:
    """The host crates nothing else lints — this scope's `-p` set.

    Host-lintable, MINUS the ones already covered:
      * `default-members`, linted by the bare `cargo clippy --all-targets`;
      * `labwired-wasm`, linted by its own step (see BROWSER_LAYER_PACKAGE).

    Both subtractions are derived — the first from the root Cargo.toml, the
    second asserted against the workflows by `--check` — so the three scopes
    partition the host set without a list anywhere.
    """
    host, _ = classify(root)
    default = set(workspace(root).get("default-members") or [])
    return [
        e["package"]
        for e in host
        if e["member"] not in default and e["package"] != BROWSER_LAYER_PACKAGE
    ]


def _clippy_invocations(root: Path) -> list[list[str]]:
    """Package lists of every `cargo clippy` line in .github/workflows/.

    An empty list means an invocation that passed no `-p` at all, i.e. the bare
    default-members form.
    """
    found: list[list[str]] = []
    workflows = root / ".github" / "workflows"
    if not workflows.is_dir():
        return found
    for path in sorted(workflows.glob("*.yml")) + sorted(workflows.glob("*.yaml")):
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.lstrip().startswith("#") or not CLIPPY_LINE.search(line):
                continue
            found.append(PACKAGE_ARG.findall(line))
    return found


def problems(root: Path) -> list[str]:
    """Everything wrong with the derived scope. Empty means the rule holds."""
    out: list[str] = []
    host, cross = classify(root)
    host_names = {e["package"] for e in host}
    invocations = _clippy_invocations(root)
    named = {pkg for inv in invocations for pkg in inv}

    if not host:
        out.append("no member classified as host-lintable — the scope is empty")
    if not cross:
        out.append(
            "no member classified as cross-target-only — with nothing in the "
            "other class the classifier is not discriminating and would pass "
            "over any tree"
        )
    if not clippy_packages(root):
        out.append(
            "this scope emits no package: every host-lintable member is either "
            "a default-member or the excused one, so the step built from it "
            "lints nothing and would report green for doing no work"
        )

    # A default-member is linted for the HOST by the bare workspace clippy that
    # has always run. If one classified as cross-target-only, the classifier is
    # wrong about the tree, not the tree about itself.
    for member in workspace(root).get("default-members") or []:
        entry = next((e for e in cross if e["member"] == member), None)
        if entry is not None:
            out.append(
                f"default-member `{member}` classified cross-target-only "
                f"({entry['reason']}), but the workspace's own "
                f"`cargo clippy --all-targets` builds it for the host"
            )

    # The half of the partition this script does NOT emit: default-members are
    # covered only for as long as a bare `cargo clippy` (no -p) still runs. If
    # somebody narrows those steps to a package list, this scope silently stops
    # being the complement it claims to be.
    if not any(inv == [] for inv in invocations):
        out.append(
            "no `cargo clippy` step in .github/workflows/ runs without `-p`, so "
            "nothing covers `default-members` any more. This scope is the "
            "COMPLEMENT of that command and does not include them."
        )

    if BROWSER_LAYER_PACKAGE not in host_names:
        out.append(
            f"`{BROWSER_LAYER_PACKAGE}` is excused from this scope on the "
            f"grounds that it is host-lintable and linted elsewhere, but it no "
            f"longer classifies as host-lintable. Drop the exception."
        )
    elif BROWSER_LAYER_PACKAGE not in named:
        out.append(
            f"`{BROWSER_LAYER_PACKAGE}` is excluded from this scope because a "
            f"dedicated `cargo clippy -p {BROWSER_LAYER_PACKAGE}` step lints it, "
            f"and no such step exists in .github/workflows/ any more. It is now "
            f"linted by nothing. Either restore that step or delete "
            f"BROWSER_LAYER_PACKAGE so this scope picks it up."
        )

    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero when the derived scope is empty, vacuous or stale",
    )
    parser.add_argument(
        "--cargo-args",
        action="store_true",
        help="print one `--package=<name>` per line for a cargo clippy invocation",
    )
    parser.add_argument("--json", action="store_true", help="print the full classification")
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="repository root to scan (default: this checkout)",
    )
    args = parser.parse_args(argv)

    try:
        _, cross = classify(args.root)
        scope = clippy_packages(args.root)
    except Unclassified as exc:
        print(
            f"host clippy scope: cannot classify a workspace member:\n  {exc}",
            file=sys.stderr,
        )
        return 1

    if args.cargo_args:
        for name in scope:
            print(f"--package={name}")
        return 0

    if args.json:
        host, _ = classify(args.root)
        print(
            json.dumps(
                {
                    "host": host,
                    "cross_target_only": cross,
                    "clippy_packages": scope,
                    "excused": BROWSER_LAYER_PACKAGE,
                },
                indent=2,
            )
        )
        return 0

    found = problems(args.root)
    if found:
        print("host clippy scope is not sound:\n", file=sys.stderr)
        for line in found:
            print(f"  {line}\n", file=sys.stderr)
        return 1

    print(
        f"host clippy scope: OK ({len(scope)} host member(s) linted by this "
        f"scope, default-members by the bare `cargo clippy`, "
        f"`{BROWSER_LAYER_PACKAGE}` by its own step, "
        f"{len(cross)} cross-target-only member(s) skipped)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
