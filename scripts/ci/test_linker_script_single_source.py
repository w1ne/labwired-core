"""Unit tests for the `-Tlink.x` single-source gate.

The gate exists because a doubled linker script is invisible on a warm
`target/`. These tests build synthetic trees, so they assert the rule itself —
including the shapes that must NOT trip it, which is where a grep-shaped check
would produce noise nobody reads.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import linker_script_single_source as gate  # noqa: E402


def make_repo(tmp_path: Path, *, build_rs: str = "", workflow: str = "", script: str = "") -> Path:
    crate = tmp_path / "crates" / "firmware-demo"
    crate.mkdir(parents=True)
    (crate / "build.rs").write_text(build_rs, encoding="utf-8")
    if workflow:
        wf = tmp_path / ".github" / "workflows"
        wf.mkdir(parents=True)
        (wf / "ci.yml").write_text(workflow, encoding="utf-8")
    if script:
        sc = tmp_path / "scripts"
        sc.mkdir(parents=True, exist_ok=True)
        (sc / "smoke.sh").write_text(script, encoding="utf-8")
    return tmp_path


OWNS = 'fn main() { println!("cargo:rustc-link-arg=-Tlink.x"); }\n'
PLAIN = 'fn main() { println!("cargo:rustc-link-search=out"); }\n'


def test_owner_plus_workflow_flag_is_a_violation(tmp_path):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow='        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo --release\n',
    )
    found = gate.violations(root)
    assert len(found) == 1
    assert "ci.yml:1" in found[0]
    assert "firmware-demo" in found[0]


def test_shell_script_is_scanned_too(tmp_path):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        script='RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
    )
    assert len(gate.violations(root)) == 1


def test_no_violation_once_the_caller_drops_the_flag(tmp_path):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow="        run: cargo build -p firmware-demo --release\n",
    )
    assert gate.violations(root) == []


def test_caller_may_pass_it_for_a_crate_that_does_not(tmp_path):
    """The flag is not banned — it is required for crates without their own.

    `firmware-rp2040-demo` is the live example: its build.rs writes memory.x but
    never emits the link arg, so the RUSTFLAGS form is the correct invocation
    and a gate that flagged it would be telling the truth backwards.
    """
    root = make_repo(
        tmp_path,
        build_rs=PLAIN,
        workflow='        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
    )
    assert gate.violations(root) == []


def test_comments_are_not_commands(tmp_path):
    """Every one of these lines exists in-tree, explaining the rule."""
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow='        # do not add RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
        script='# was: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
    )
    assert gate.violations(root) == []


def test_a_commented_out_build_rs_line_does_not_make_an_owner(tmp_path):
    root = make_repo(
        tmp_path,
        build_rs='fn main() { /* println!("cargo:rustc-link-arg=-Tlink.x"); */ }\n',
        workflow='        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
    )
    assert gate.self_linking_crates(root) == {}
    assert gate.violations(root) == []


@pytest.mark.parametrize("form", ["-p firmware-demo", "-p=firmware-demo", "--package firmware-demo"])
def test_every_package_spelling_is_caught(tmp_path, form):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow=f'        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build {form} --release\n',
    )
    assert len(gate.violations(root)) == 1


def test_a_different_crate_on_the_same_line_is_not_flagged(tmp_path):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow='        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-other\n',
    )
    assert gate.violations(root) == []


def test_main_returns_nonzero_and_names_the_file(tmp_path, capsys):
    root = make_repo(
        tmp_path,
        build_rs=OWNS,
        workflow='        run: RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-demo\n',
    )
    assert gate.main(["--check", "--root", str(root)]) == 1
    assert "ci.yml" in capsys.readouterr().err


def test_this_repository_passes():
    """The live tree. This is the assertion that would have caught the break."""
    assert gate.violations(gate.REPO_ROOT) == []


def test_the_live_tree_has_owners_to_check():
    """A gate over an empty owner set would pass by finding nothing to check."""
    assert gate.self_linking_crates(gate.REPO_ROOT)
